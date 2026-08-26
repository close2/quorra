//! The compute coverage lane: `raster/fill.rs`'s exact arithmetic, run by the device
//! (ADR 0079, ADR 0080).
//!
//! One subject in two halves, shaped as [`crate::winding`] is. The encoder builds a
//! [`ComputeSheet`] while it walks — every tile's edges in tile space, in encounter
//! order — and prices it *before* anything is allocated, because a buffer sized from
//! document-derived arithmetic is what principle 3 says to check first. The device then
//! runs one dispatch: one invocation per tile row, each depositing its row's exact
//! signed trapezoid areas, prefix-summing, quantising, and writing its bytes into the
//! frame's coverage image.
//!
//! # Why the bytes can be the CPU's bytes
//!
//! The shader is a statement-for-statement port of [`crate::raster::fill_mask`], and the
//! port's determinism was measured before this lane was built
//! (`tests/compute_coverage_determinism.rs`, ADR 0079): byte-identical to the CPU
//! arithmetic on every adapter this machine has, both fill rules, through every branch.
//! The row-parallel shape preserves the CPU's per-cell deposit order because `fill.rs`
//! interpolates every slab from the edge's top rather than incrementally — the property
//! that makes the port order-*identical*, not merely order-independent. The four named
//! hazards and their answers live as comments in the WGSL below.
//!
//! # How the bytes reach the sheet
//!
//! The scratch sheet's texels are bytes, rows of different tiles can share a 32-bit
//! word, and a non-atomic read-modify-write across invocations would race — so the
//! shader ORs each byte into a zero base with `atomicOr`, which is deterministic
//! because every byte has exactly one writer and OR against zero is assignment. The
//! frame's image travels texture → buffer → dispatch → texture: the pre-copy seeds the
//! buffer with the CPU lane's tiles (and zeros elsewhere, which wgpu's zero
//! initialisation guarantees), the dispatch fills the compute tiles' bytes, and one
//! buffer-to-texture copy puts the whole sheet back. Two full-sheet copies instead of a
//! copy per tile, which is ADR 0078's lesson standing.

/// One tile's record, as the encoder placed it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ComputeTile {
    /// Where the tile sits on the scratch sheet.
    pub seat: [u32; 2],
    pub width: u32,
    pub height: u32,
    /// §8.5.3.3's rule: `false` non-zero, `true` even-odd.
    pub even_odd: bool,
    /// The tile's run of [`ComputeSheet::edges`], in edges (four floats each).
    pub edge_start: u32,
    pub edge_count: u32,
    /// The tile's run of the accumulator buffer, in floats: `(width + 1) × height`.
    pub acc_start: u32,
    /// The tile's first row job, so an invocation can name its local row.
    pub row_start: u32,
}

/// The number of `u32`s one tile occupies in the GPU-side tile buffer.
const TILE_U32S: usize = 9;

/// What the encoder built for the compute lane this frame.
#[derive(Debug, Default)]
pub(crate) struct ComputeSheet {
    /// Every tile's edges — `x0, y0, x1, y1` per edge, already shifted into tile space
    /// with exactly `fill_mask`'s subtraction so the arithmetic downstream sees the
    /// same bits.
    pub edges: Vec<f32>,
    pub tiles: Vec<ComputeTile>,
    /// Total row jobs: the sum of every tile's height.
    pub rows: u32,
    /// Total accumulator floats: the sum of every tile's `(width + 1) × height`.
    pub acc_floats: u64,
    /// The scratch sheet's extent, filled in by `encoded::finish` exactly as the
    /// winding sheet's is.
    pub width: u32,
    pub height: u32,
}

impl ComputeSheet {
    /// Adds a tile: its seat, its extent, and its edges as the worker extracted them.
    ///
    /// Returns `false` — and records nothing — when a counter would saturate; the
    /// caller then falls back to the CPU lane for this tile, which draws the same
    /// bytes. In practice unreachable: the frame budget refuses such a frame first.
    #[allow(clippy::cast_possible_truncation)] // guarded: every count is checked below
    #[allow(clippy::arithmetic_side_effects)] // the additions run only after the
    // headroom checks above them, which is what the checks are for
    pub(crate) fn push_tile(
        &mut self,
        seat: (u32, u32),
        width: u32,
        height: u32,
        even_odd: bool,
        edges: &[f32],
    ) -> bool {
        let edge_start = self.edges.len() / 4;
        let edge_count = edges.len() / 4;
        let acc = u64::from(width.saturating_add(1)).saturating_mul(u64::from(height));
        let (Ok(edge_start), Ok(edge_count), Ok(acc_start)) = (
            u32::try_from(edge_start),
            u32::try_from(edge_count),
            u32::try_from(self.acc_floats),
        ) else {
            return false;
        };
        if self.acc_floats.saturating_add(acc) > u64::from(u32::MAX)
            || u64::from(self.rows).saturating_add(u64::from(height)) > u64::from(u32::MAX)
        {
            return false;
        }
        self.tiles.push(ComputeTile {
            seat: [seat.0, seat.1],
            width,
            height,
            even_odd,
            edge_start,
            edge_count,
            acc_start,
            row_start: self.rows,
        });
        self.edges.extend_from_slice(edges);
        self.rows += height;
        self.acc_floats += acc;
        true
    }

    /// Whether this frame drew anything through the compute lane.
    pub(crate) fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// The image buffer's row stride in bytes: the sheet's width rounded up to what a
    /// buffer-to-texture copy requires.
    fn stride(&self) -> u64 {
        u64::from(self.width).div_ceil(256).saturating_mul(256)
    }

    /// Bytes this sheet costs on the device, for the frame budget — counted before
    /// anything is allocated (principle 3), and zero when no tile took the lane, which
    /// is exactly the condition the device allocates under
    /// ([`Sheet::device_bytes`](crate::winding::Sheet::device_bytes)'s discipline).
    #[allow(clippy::cast_possible_truncation)] // lengths of Vecs this frame just built
    pub(crate) fn device_bytes(&self) -> u64 {
        if self.is_empty() {
            return 0;
        }
        let edges = (self.edges.len() as u64).saturating_mul(4);
        let tiles = (self.tiles.len() as u64)
            .saturating_mul(TILE_U32S as u64)
            .saturating_mul(4);
        let rows = u64::from(self.rows).saturating_mul(4);
        let acc = self.acc_floats.saturating_mul(4);
        let image = self.stride().saturating_mul(u64::from(self.height));
        edges
            .saturating_add(tiles)
            .saturating_add(rows)
            .saturating_add(acc)
            .saturating_add(image)
    }

    /// Heap bytes the encoder-side record holds, for a retained encode's price.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn retained_bytes(&self) -> u64 {
        (self.edges.len() as u64).saturating_mul(4).saturating_add(
            (self.tiles.len() as u64).saturating_mul(size_of::<ComputeTile>() as u64),
        )
    }
}

/// The shader: one invocation per tile row. See the module comment for the argument
/// that its bytes are the CPU rasteriser's bytes, and
/// `tests/compute_coverage_determinism.rs` for the measurement.
const SHADER: &str = r"
struct Params {
    stride_words: u32,
    rows: u32,
    tile_stride: u32,
    padding: u32,
}

@group(0) @binding(0) var<storage, read> edges: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read> tiles: array<u32>;
@group(0) @binding(2) var<storage, read> row_jobs: array<u32>;
@group(0) @binding(3) var<storage, read_write> acc: array<f32>;
@group(0) @binding(4) var<storage, read_write> image: array<atomic<u32>>;
@group(0) @binding(5) var<uniform> params: Params;

// fill.rs's deposit_inside: split at each vertical cell boundary, exact trapezoid
// areas into the row accumulator at `base`.
fn deposit_inside(base: u32, fw: f32, dir: f32, xs_in: f32, ys: f32, xe_in: f32, ye: f32) {
    let xs = clamp(xs_in, 0.0, fw);
    let xe = clamp(xe_in, 0.0, fw);
    var px = xs;
    var py = ys;
    loop {
        var has_boundary = false;
        var b = 0.0;
        if (xe > px) {
            b = floor(px) + 1.0;
            has_boundary = b < xe;
        } else if (xe < px) {
            b = ceil(px) - 1.0;
            has_boundary = b > xe;
        }
        var nx = xe;
        var ny = ye;
        if (has_boundary) {
            let t = (b - xs) / (xe - xs);
            nx = b;
            ny = ys + (ye - ys) * t;
        }
        let d = dir * (ny - py);
        if (d != 0.0) {
            let xm = 0.5 * (px + nx);
            let cell = u32(clamp(floor(xm), 0.0, fw - 1.0));
            let frac = xm - f32(cell);
            acc[base + cell] += d * (1.0 - frac);
            acc[base + cell + 1u] += d * frac;
        }
        if (!has_boundary) {
            break;
        }
        px = nx;
        py = ny;
    }
}

// fill.rs's deposit_slab: the part of a slab spent outside the region is cut off at
// the border and deposited there (ADR 0049's rule, ported bit for bit).
fn deposit_slab(base: u32, fw: f32, dir: f32, xs: f32, ys: f32, xe: f32, ye: f32) {
    if (xs >= 0.0 && xs <= fw && xe >= 0.0 && xe <= fw) {
        deposit_inside(base, fw, dir, xs, ys, xe, ye);
        return;
    }
    let dx = xe - xs;
    let dy = ye - ys;
    var cut_t = array<f32, 2>(0.0, 0.0);
    var cut_b = array<f32, 2>(0.0, 0.0);
    var count = 0u;
    if (dx != 0.0) {
        for (var i = 0u; i < 2u; i += 1u) {
            let border = select(0.0, fw, i == 1u);
            let t = (border - xs) / dx;
            if (t > 0.0 && t < 1.0) {
                cut_t[count] = t;
                cut_b[count] = border;
                count += 1u;
            }
        }
        if (count == 2u && cut_t[1] < cut_t[0]) {
            let tt = cut_t[0]; cut_t[0] = cut_t[1]; cut_t[1] = tt;
            let bb = cut_b[0]; cut_b[0] = cut_b[1]; cut_b[1] = bb;
        }
    }
    var px = xs;
    var py = ys;
    for (var i = 0u; i < count; i += 1u) {
        let nx = cut_b[i];
        let ny = ys + dy * cut_t[i];
        deposit_inside(base, fw, dir, px, py, nx, ny);
        px = nx;
        py = ny;
    }
    deposit_inside(base, fw, dir, px, py, xe, ye);
}

@compute @workgroup_size(64)
fn coverage_rows(@builtin(global_invocation_id) id: vec3<u32>) {
    let job = id.y * 65536u + id.x;
    if (job >= params.rows) { return; }
    let tile = row_jobs[job];
    let t = tile * params.tile_stride;
    let seat_x = tiles[t];
    let seat_y = tiles[t + 1u];
    let width = tiles[t + 2u];
    let even_odd = tiles[t + 4u];
    let edge_start = tiles[t + 5u];
    let edge_count = tiles[t + 6u];
    let acc_start = tiles[t + 7u];
    let row_start = tiles[t + 8u];
    let my_row = job - row_start;
    let height = tiles[t + 3u];
    let fw = f32(width);
    let fh = f32(height);
    let fy = f32(my_row);
    let base = acc_start + my_row * (width + 1u);
    // The accumulator arrives zeroed (a fresh buffer, which WebGPU zero-initialises).
    for (var e = 0u; e < edge_count; e += 1u) {
        let edge = edges[edge_start + e];
        let x0 = edge.x; let y0 = edge.y; let x1 = edge.z; let y1 = edge.w;
        // Exact comparison, as fill.rs: a horizontal edge deposits nothing.
        if (y0 == y1) { continue; }
        var dir = 1.0;
        var top_x = x0; var top_y = y0; var bot_x = x1; var bot_y = y1;
        if (y0 >= y1) {
            dir = -1.0;
            top_x = x1; top_y = y1; bot_x = x0; bot_y = y0;
        }
        if (top_y < 0.0) {
            top_x = top_x + (bot_x - top_x) * (0.0 - top_y) / (bot_y - top_y);
            top_y = 0.0;
        }
        if (bot_y > fh) {
            bot_x = top_x + (bot_x - top_x) * (fh - top_y) / (bot_y - top_y);
            bot_y = fh;
        }
        if (bot_y <= top_y) { continue; }
        let dxdy = (bot_x - top_x) / (bot_y - top_y);
        // fill.rs guards !dxdy.is_finite(); WGSL may assume no NaN or infinity is
        // produced, so the guard is the magnitude the comment there derives: a slope
        // past f32's range is a slab under 2.4e-11 of a pixel, whose exact deposit is
        // nothing to eleven decimal places.
        if (abs(dxdy) > 3.0e38) { continue; }
        // The CPU walks y from max(floor(top_y), 0) to bot_y in steps of one; this row
        // participates exactly when it lies in that integer sequence.
        if (fy < max(floor(top_y), 0.0) || fy >= bot_y) { continue; }
        let entry_y = max(top_y, fy);
        let exit_y = min(bot_y, fy + 1.0);
        let entry_x = top_x + (entry_y - top_y) * dxdy;
        let exit_x = top_x + (exit_y - top_y) * dxdy;
        deposit_slab(base, fw, dir, entry_x, entry_y, exit_x, exit_y);
    }
    var running = 0.0;
    let row_word = (seat_y + my_row) * params.stride_words;
    for (var x = 0u; x < width; x += 1u) {
        running += acc[base + x];
        var cov = 0.0;
        if (even_odd == 0u) {
            cov = min(abs(running), 1.0);
        } else {
            // rem_euclid(2.0) as x - 2*floor(x/2), exact in f32 for x >= 0 (Sterbenz);
            // WGSL's own % rounds twice and is not used.
            let a = abs(running);
            let m = a - 2.0 * floor(a * 0.5);
            cov = 1.0 - abs(m - 1.0);
        }
        // Rust's round is ties-away-from-zero and coverage is non-negative, so
        // floor(x + 0.5) is that rule; WGSL's round() is ties-to-even and is not used.
        let value = u32(floor(cov * 255.0 + 0.5));
        let byte = seat_x + x;
        atomicOr(&image[row_word + byte / 4u], value << (8u * (byte % 4u)));
    }
}
";

/// The per-frame buffers of one dispatch, alive until the submit.
struct Buffers {
    tiles: wgpu::Buffer,
    edges: wgpu::Buffer,
    row_jobs: wgpu::Buffer,
    acc: wgpu::Buffer,
    image: wgpu::Buffer,
    params: wgpu::Buffer,
}

/// Runs the lane: seeds the image buffer from the sheet texture, dispatches one
/// invocation per tile row, and copies the whole image back.
///
/// The pipeline is compiled on the first frame that takes the lane and kept — the
/// startup path never pays for it (§7) — inside a validation scope so a refusal is a
/// typed error rather than a panic elsewhere.
///
/// A driver that refuses the shader or the pipeline says so through the device's
/// uncaptured-error channel, exactly as a warm-set pipeline's refusal does.
pub(crate) fn dispatch_into(
    gpu: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &mut Option<wgpu::ComputePipeline>,
    sheet: &wgpu::Texture,
    compute: &ComputeSheet,
) {
    if pipeline.is_none() {
        let module = gpu.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quorra compute coverage"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        *pipeline = Some(
            gpu.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("quorra compute coverage"),
                layout: None,
                module: &module,
                entry_point: Some("coverage_rows"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            }),
        );
    }
    let Some(pipeline) = pipeline.as_ref() else {
        return; // unreachable: assigned above
    };
    let buffers = make_buffers(gpu, queue, compute);
    let group = gpu.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("quorra compute coverage"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            entry(0, &buffers.edges),
            entry(1, &buffers.tiles),
            entry(2, &buffers.row_jobs),
            entry(3, &buffers.acc),
            entry(4, &buffers.image),
            entry(5, &buffers.params),
        ],
    });
    #[allow(clippy::cast_possible_truncation)] // the stride is the sheet's width
    // rounded to 256, and the sheet is bounded by the device's texture dimension
    let stride = compute.stride() as u32;
    let mut encoder = gpu.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("quorra compute coverage"),
    });
    // Seed: the CPU lane's tiles (queued by `write_texture`, which precedes this
    // submit) and zeros everywhere else, so the OR below is assignment.
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: sheet,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffers.image,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: compute.width,
            height: compute.height,
            depth_or_array_layers: 1,
        },
    );
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("quorra compute coverage"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        // 1024 groups of 64 along x, and as many y rows of that as the jobs need: the
        // job index is `y * 65536 + x`, so no dispatch dimension can overflow.
        pass.dispatch_workgroups(1024, compute.rows.div_ceil(65536), 1);
    }
    encoder.copy_buffer_to_texture(
        wgpu::TexelCopyBufferInfo {
            buffer: &buffers.image,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        wgpu::TexelCopyTextureInfo {
            texture: sheet,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width: compute.width,
            height: compute.height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
}

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

/// The dispatch's buffers, sized by what the encoder counted.
#[allow(clippy::cast_possible_truncation)] // stride fits u32: it is the sheet's width
// rounded to 256, and the sheet is bounded by the device's texture dimension
fn make_buffers(gpu: &wgpu::Device, queue: &wgpu::Queue, compute: &ComputeSheet) -> Buffers {
    let storage = |label: &str, bytes: &[u8]| {
        let buffer = gpu.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes.len().max(16) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytes);
        buffer
    };
    let mut tile_words: Vec<u8> = Vec::with_capacity(
        compute
            .tiles
            .len()
            .saturating_mul(TILE_U32S)
            .saturating_mul(4),
    );
    let mut row_jobs: Vec<u8> = Vec::with_capacity((compute.rows as usize).saturating_mul(4));
    for (index, tile) in compute.tiles.iter().enumerate() {
        for word in [
            tile.seat[0],
            tile.seat[1],
            tile.width,
            tile.height,
            u32::from(tile.even_odd),
            tile.edge_start,
            tile.edge_count,
            tile.acc_start,
            tile.row_start,
        ] {
            tile_words.extend_from_slice(&word.to_le_bytes());
        }
        let index = u32::try_from(index).unwrap_or(u32::MAX);
        for _ in 0..tile.height {
            row_jobs.extend_from_slice(&index.to_le_bytes());
        }
    }
    let edge_bytes: Vec<u8> = compute
        .edges
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let stride = compute.stride();
    let image = gpu.create_buffer(&wgpu::BufferDescriptor {
        label: Some("quorra compute coverage image"),
        size: stride.saturating_mul(u64::from(compute.height)).max(16),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let acc = gpu.create_buffer(&wgpu::BufferDescriptor {
        label: Some("quorra compute coverage accumulator"),
        size: compute.acc_floats.saturating_mul(4).max(16),
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let params: Vec<u8> = [stride as u32 / 4, compute.rows, TILE_U32S as u32, 0]
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect();
    let params_buffer = gpu.create_buffer(&wgpu::BufferDescriptor {
        label: Some("quorra compute coverage params"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&params_buffer, 0, &params);
    Buffers {
        tiles: storage("quorra compute coverage tiles", &tile_words),
        edges: storage("quorra compute coverage edges", &edge_bytes),
        row_jobs: storage("quorra compute coverage rows", &row_jobs),
        acc,
        image,
        params: params_buffer,
    }
}

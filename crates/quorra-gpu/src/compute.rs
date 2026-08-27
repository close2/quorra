//! The compute coverage lane: flattening and `raster/fill.rs`'s exact scanline
//! arithmetic, both run by the device from resident outlines (ADR 0079, 0080, 0081).
//!
//! One subject in two halves, shaped as [`crate::winding`] is. The encoder records one
//! [`TileRecord`] per solid fill — its seat, its rectangle, its transform, and *which
//! outline* — and nothing else: no flattening, no edges, no per-frame geometry on the
//! host. The device keeps every outline's segments in one resident arena
//! ([`SegmentArena`]), uploaded on the first frame that draws it, and turns the
//! records into coverage in three passes:
//!
//! 1. **count** — one invocation per tile walks its outline's segments under its
//!    transform, running the flattening arithmetic and counting the edges it would
//!    emit;
//! 2. **emit** — after an exact allocation from those counts (one readback, which is
//!    what "count then allocate" costs when the counter is the device), the same walk
//!    writes the closed edge lists in tile space;
//! 3. **deposit** — one invocation per tile row runs the scanline port: exact signed
//!    trapezoid areas, the serial prefix sum, the CPU's own rounding, bytes OR'd into
//!    the zero-seeded sheet image (deterministic: every byte has exactly one writer,
//!    and OR against zero is assignment). The image travels texture → buffer →
//!    passes → texture, so a mixed sheet costs two whole copies and never one per
//!    tile (ADR 0078's lesson standing).
//!
//! # Why the bytes are still the CPU's bytes
//!
//! Every stage is the same arithmetic in the same order. The flattening is
//! `raster/flatten.rs` statement for statement — the transform's `a·x + c·y + e`, the
//! exact midpoint halving, the flatness cross-products, the depth cap — made iterative
//! with an explicit stack because WGSL has no recursion, pushed right-half-first so
//! the emission order is the recursion's; the cubic's own control points stay in
//! unshifted device space so no round-trip through the tile shift can move a bit. The
//! deposit pass is ADR 0080's shader unchanged. The **one stated divergence**:
//! `cubic_tolerance` takes `√(w² + h²)` where the CPU takes `f32::hypot`, which WGSL
//! does not have — the two differ by at most an ulp of the diagonal, that can matter
//! only for a cubic whose flatness test lands within it of the boundary, and
//! `tests/compute_lane.rs` holds whole frames byte-equal over the fixtures anyway. If
//! a fixture ever finds the boundary, the resolution is ADR 0077's: share the
//! arithmetic, by its own ADR.
//!
//! # What the readback costs and buys
//!
//! The count pass's totals cross back to the host once per frame, so the edge buffer
//! is allocated *exactly* and checked against the frame budget before it exists —
//! principle 3 with the device as the counter, and principle 6's refusal with a name
//! when a magnification makes more edges than the budget holds. The price is one
//! submit boundary mid-frame; replacing it with a fence and a retry is a measurement
//! for the round that needs it.

use crate::error::RenderError;
use crate::keyhash::FastMap;
use quorra_scene::Segment;

/// One tile's record, as the encoder placed it: everything the device needs to
/// rasterise one solid fill, and nothing the frame's order decides.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TileRecord {
    /// Where the tile sits on the scratch sheet.
    pub seat: [u32; 2],
    /// The tile's device corner — the same `i32 → f32` cast the CPU rasteriser makes.
    pub left: f32,
    pub top: f32,
    pub width: u32,
    pub height: u32,
    /// §8.5.3.3's rule: `false` non-zero, `true` even-odd.
    pub even_odd: bool,
    /// The composed device transform the outline is flattened under.
    pub transform: [f32; 6],
    /// The outline whose segments the arena holds, by raw id.
    pub outline: u32,
    /// The tile's run of the accumulator buffer, in floats: `(width + 1) × height`.
    pub acc_start: u32,
    /// The tile's first row job, so a deposit invocation can name its local row.
    pub row_start: u32,
}

/// The number of `u32`s one tile occupies in the GPU-side tile buffer: seat and
/// extent, the rule, the arena range, the accumulator and row starts, a reserved
/// word, the corner, and the six transform coefficients — padded to a round stride.
const TILE_U32S: usize = 20;

/// What the encoder built for the compute lane this frame.
#[derive(Debug, Default)]
pub(crate) struct ComputeSheet {
    pub tiles: Vec<TileRecord>,
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
    /// Adds a tile. Returns `false` — and records nothing — when a counter would
    /// saturate; unreachable for any frame the byte budget admits, and the caller
    /// refuses rather than guessing when it is not.
    #[allow(clippy::arithmetic_side_effects)] // the additions run only after the
    // headroom checks above them, which is what the checks are for
    pub(crate) fn push_tile(
        &mut self,
        seat: (u32, u32),
        tile: (f32, f32, u32, u32),
        even_odd: bool,
        transform: [f32; 6],
        outline: u32,
    ) -> bool {
        let (left, top, width, height) = tile;
        let acc = u64::from(width.saturating_add(1)).saturating_mul(u64::from(height));
        let Ok(acc_start) = u32::try_from(self.acc_floats) else {
            return false;
        };
        if self.acc_floats.saturating_add(acc) > u64::from(u32::MAX)
            || u64::from(self.rows).saturating_add(u64::from(height)) > u64::from(u32::MAX)
        {
            return false;
        }
        self.tiles.push(TileRecord {
            seat: [seat.0, seat.1],
            left,
            top,
            width,
            height,
            even_odd,
            transform,
            outline,
            acc_start,
            row_start: self.rows,
        });
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
    /// anything is allocated, and zero when no tile took the lane. **The edge buffer
    /// is deliberately absent**: its size is the count pass's answer, checked against
    /// the same budget at allocation ([`dispatch_into`]). The resident arena is
    /// charged to the resource side, where residency lives.
    #[allow(clippy::cast_possible_truncation)] // lengths of Vecs this frame just built
    pub(crate) fn device_bytes(&self) -> u64 {
        if self.is_empty() {
            return 0;
        }
        let tiles = (self.tiles.len() as u64)
            .saturating_mul(TILE_U32S as u64)
            .saturating_mul(4);
        let counts = (self.tiles.len() as u64).saturating_mul(8); // counts + offsets
        let acc = self.acc_floats.saturating_mul(4);
        let image = self.stride().saturating_mul(u64::from(self.height));
        tiles
            .saturating_add(counts)
            .saturating_add(acc)
            .saturating_add(image)
    }

    /// Heap bytes the encoder-side record holds, for a retained encode's price.
    pub(crate) fn retained_bytes(&self) -> u64 {
        (self.tiles.len() as u64).saturating_mul(size_of::<TileRecord>() as u64)
    }
}

/// The resident segment arena: every outline the lane has drawn, in one buffer,
/// uploaded once (ADR 0081).
///
/// Grows by doubling with a device-side copy, so residency survives its own growth. A
/// released outline's entry leaves the map — its id may be reissued, and a stale range
/// would be another outline's geometry — but its words stay until the device goes;
/// [`SegmentArena::holes`] counts them so the cost is a number rather than a feeling,
/// and compaction is a measured decision for the round that sees it grow.
#[derive(Debug, Default)]
pub(crate) struct SegmentArena {
    buffer: Option<wgpu::Buffer>,
    /// Words used and words allocated.
    used: u64,
    capacity: u64,
    /// Raw outline id → (word offset, word length).
    entries: FastMap<u32, (u32, u32)>,
    /// Words belonging to released outlines, unreachable until the device goes.
    pub holes: u64,
}

/// One outline's arena encoding: a tag word per segment, then its point words.
fn encode_segments(segments: &[Segment], out: &mut Vec<u32>) {
    for segment in segments {
        match *segment {
            Segment::MoveTo(p) => out.extend_from_slice(&[0, p.x.to_bits(), p.y.to_bits()]),
            Segment::LineTo(p) => out.extend_from_slice(&[1, p.x.to_bits(), p.y.to_bits()]),
            Segment::CubicTo { c1, c2, to } => out.extend_from_slice(&[
                2,
                c1.x.to_bits(),
                c1.y.to_bits(),
                c2.x.to_bits(),
                c2.y.to_bits(),
                to.x.to_bits(),
                to.y.to_bits(),
            ]),
            Segment::Close => out.push(3),
        }
    }
}

impl SegmentArena {
    /// The arena range holding this outline's segments, uploading them on first use.
    ///
    /// `None` when the encoded outline alone would overflow a `u32` word range, which
    /// the resource budget refuses long before it is reachable.
    pub(crate) fn ensure(
        &mut self,
        gpu: &wgpu::Device,
        queue: &wgpu::Queue,
        outline: u32,
        segments: &[Segment],
    ) -> Option<(u32, u32)> {
        if let Some(range) = self.entries.get(&outline) {
            return Some(*range);
        }
        let mut words = Vec::new();
        encode_segments(segments, &mut words);
        let len = u32::try_from(words.len()).ok()?;
        let offset = u32::try_from(self.used).ok()?;
        let needed = self.used.saturating_add(u64::from(len)).saturating_mul(4);
        if needed > self.capacity {
            self.grow(gpu, queue, needed);
        }
        let buffer = self.buffer.as_ref()?;
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        queue.write_buffer(buffer, self.used.saturating_mul(4), &bytes);
        self.used = self.used.saturating_add(u64::from(len));
        self.entries.insert(outline, (offset, len));
        Some((offset, len))
    }

    /// Forgets a released outline's range (`Device::release`); its words become holes.
    pub(crate) fn forget(&mut self, outline: u32) {
        if let Some((_, len)) = self.entries.remove(&outline) {
            self.holes = self.holes.saturating_add(u64::from(len));
        }
    }

    /// Bytes the arena holds on the device.
    pub(crate) fn device_bytes(&self) -> u64 {
        self.capacity
    }

    fn grow(&mut self, gpu: &wgpu::Device, queue: &wgpu::Queue, needed: u64) {
        let capacity = needed.next_power_of_two().max(1 << 20);
        let grown = gpu.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quorra segment arena"),
            size: capacity,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        if let Some(old) = self.buffer.take()
            && self.used > 0
        {
            let mut encoder = gpu.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("quorra segment arena growth"),
            });
            encoder.copy_buffer_to_buffer(&old, 0, &grown, 0, self.used.saturating_mul(4));
            queue.submit([encoder.finish()]);
        }
        self.buffer = Some(grown);
        self.capacity = capacity;
    }
}

/// The flattening walk, one WGSL text shared by the count and emit passes — two
/// compiled forms differing only in the `EMIT` constant prepended by
/// [`Pipelines::compile`], so the two passes cannot disagree about a count.
const FLATTEN: &str = r"
struct Params {
    stride_words: u32,
    rows: u32,
    tiles: u32,
    // The edges buffer's persistent capacity, in edges (ADR 0095): what the emit
    // guards every write against, since its offsets come from the device's own scan
    // and the host has not seen a total.
    capacity: u32,
}

@group(0) @binding(0) var<storage, read> arena: array<u32>;
@group(0) @binding(1) var<storage, read> tiles: array<u32>;
@group(0) @binding(2) var<storage, read_write> counts: array<u32>;
@group(0) @binding(3) var<storage, read> offsets: array<u32>;
@group(0) @binding(4) var<storage, read_write> edges: array<vec4<f32>>;
@group(0) @binding(5) var<uniform> params: Params;
// The overflow flag (ADR 0095): raised where an emit met the capacity, read on the
// host at the frame's own end-wait, before anything is presented. Referenced only
// under EMIT; the count variants bind a placeholder because the layout keeps a
// const-dead binding.
@group(0) @binding(6) var<storage, read_write> overflow: array<u32>;

const TILE_U32S: u32 = 20u;
const FLATTEN_TOLERANCE: f32 = 0.25;
const RELATIVE_FLATTEN_TOLERANCE: f32 = 0.03125;

struct Walk {
    // The transform, the tile shift, and the running polyline state. The previous
    // point is kept in both forms: shifted for the edge it starts, unshifted for the
    // cubic that continues from it — a round-trip through the shift could move a bit.
    a: f32, b: f32, c: f32, d: f32, e: f32, f: f32,
    left: f32, top: f32,
    first: vec2<f32>,
    prev_shifted: vec2<f32>,
    prev_device: vec2<f32>,
    count: u32,
    cursor: u32,
    emitted: u32,
}

fn apply(w: ptr<function, Walk>, x: f32, y: f32) -> vec2<f32> {
    return vec2<f32>(
        (*w).a * x + (*w).c * y + (*w).e,
        (*w).b * x + (*w).d * y + (*w).f,
    );
}

fn edge(w: ptr<function, Walk>, tail: vec2<f32>, head: vec2<f32>) {
    if (EMIT) {
        // Guarded against the persistent capacity (ADR 0095): an overflow stops
        // writing, raises the flag, and the frame re-runs grown before any present —
        // a wrong picture is impossible by construction, only a slower frame.
        if ((*w).cursor < params.capacity) {
            edges[(*w).cursor] = vec4<f32>(tail.x, tail.y, head.x, head.y);
            (*w).cursor += 1u;
        } else {
            overflow[0] = 1u;
        }
    }
    (*w).emitted += 1u;
}

// A device-space point joins the current polyline: shifted into tile space with
// fill_mask's own subtraction, an edge from its predecessor.
fn point(w: ptr<function, Walk>, p: vec2<f32>) {
    let shifted = vec2<f32>(p.x - (*w).left, p.y - (*w).top);
    if ((*w).count == 0u) {
        (*w).first = shifted;
    } else {
        edge(w, (*w).prev_shifted, shifted);
    }
    (*w).prev_shifted = shifted;
    (*w).prev_device = p;
    (*w).count += 1u;
}

// The subpath ends: closed by the fill exactly as fill_mask's modular walk closes it,
// unless it holds one point or none (flatten.rs keeps only polylines of two or more).
fn flush(w: ptr<function, Walk>) {
    if ((*w).count > 1u) {
        edge(w, (*w).prev_shifted, (*w).first);
    }
    (*w).count = 0u;
}

fn cubic_tolerance(p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>, p3: vec2<f32>) -> f32 {
    let width = max(max(p0.x, p1.x), max(p2.x, p3.x)) - min(min(p0.x, p1.x), min(p2.x, p3.x));
    let height = max(max(p0.y, p1.y), max(p2.y, p3.y)) - min(min(p0.y, p1.y), min(p2.y, p3.y));
    // flatten.rs takes f32::hypot here; WGSL has none, and the module comment carries
    // the one-ulp argument for this being the lane's one stated divergence.
    return min(FLATTEN_TOLERANCE, RELATIVE_FLATTEN_TOLERANCE * sqrt(width * width + height * height));
}

fn is_flat(p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>, p3: vec2<f32>, tolerance: f32) -> bool {
    let dx = p3.x - p0.x;
    let dy = p3.y - p0.y;
    let d1 = abs((p1.x - p0.x) * dy - (p1.y - p0.y) * dx);
    let d2 = abs((p2.x - p0.x) * dy - (p2.y - p0.y) * dx);
    let len_sq = dx * dx + dy * dy;
    if (len_sq <= 1.1920929e-7) {
        let c1 = max(abs(p1.x - p0.x), abs(p1.y - p0.y));
        let c2 = max(abs(p2.x - p0.x), abs(p2.y - p0.y));
        return max(c1, c2) <= tolerance;
    }
    let d = max(d1, d2);
    return d * d <= tolerance * tolerance * len_sq;
}

// flatten.rs's recursion, iterative: the right half is pushed first so the left is
// popped first, which is the recursion's emission order. Depth 16 caps the stack.
fn flatten_cubic(w: ptr<function, Walk>, start: vec2<f32>, c1: vec2<f32>, c2: vec2<f32>, to: vec2<f32>) {
    let tolerance = cubic_tolerance(start, c1, c2, to);
    var stack_p0: array<vec2<f32>, 17>;
    var stack_p1: array<vec2<f32>, 17>;
    var stack_p2: array<vec2<f32>, 17>;
    var stack_p3: array<vec2<f32>, 17>;
    var stack_depth: array<u32, 17>;
    var sp = 1u;
    stack_p0[0] = start; stack_p1[0] = c1; stack_p2[0] = c2; stack_p3[0] = to;
    stack_depth[0] = 0u;
    while (sp > 0u) {
        sp -= 1u;
        let p0 = stack_p0[sp]; let p1 = stack_p1[sp];
        let p2 = stack_p2[sp]; let p3 = stack_p3[sp];
        let depth = stack_depth[sp];
        if (is_flat(p0, p1, p2, p3, tolerance) || depth >= 16u) {
            point(w, p3);
            continue;
        }
        let q0 = (p0 + p1) * 0.5;
        let q1 = (p1 + p2) * 0.5;
        let q2 = (p2 + p3) * 0.5;
        let r0 = (q0 + q1) * 0.5;
        let r1 = (q1 + q2) * 0.5;
        let split = (r0 + r1) * 0.5;
        stack_p0[sp] = split; stack_p1[sp] = r1; stack_p2[sp] = q2; stack_p3[sp] = p3;
        stack_depth[sp] = depth + 1u;
        sp += 1u;
        stack_p0[sp] = p0; stack_p1[sp] = q0; stack_p2[sp] = r0; stack_p3[sp] = split;
        stack_depth[sp] = depth + 1u;
        sp += 1u;
    }
}

@compute @workgroup_size(64)
fn flatten_tiles(@builtin(global_invocation_id) id: vec3<u32>) {
    let tile = id.y * 65536u + id.x;
    if (tile >= params.tiles) { return; }
    let t = tile * TILE_U32S;
    var w: Walk;
    w.left = bitcast<f32>(tiles[t + 10u]);
    w.top = bitcast<f32>(tiles[t + 11u]);
    w.a = bitcast<f32>(tiles[t + 12u]);
    w.b = bitcast<f32>(tiles[t + 13u]);
    w.c = bitcast<f32>(tiles[t + 14u]);
    w.d = bitcast<f32>(tiles[t + 15u]);
    w.e = bitcast<f32>(tiles[t + 16u]);
    w.f = bitcast<f32>(tiles[t + 17u]);
    w.count = 0u;
    w.emitted = 0u;
    w.cursor = select(0u, offsets[tile], EMIT);
    let seg_offset = tiles[t + 5u];
    let seg_end = seg_offset + tiles[t + 6u];
    var s = seg_offset;
    // `count == 0` doubles as flatten.rs's `current.is_empty()`: a LineTo or CubicTo
    // before any MoveTo is ignored because no polyline is open.
    while (s < seg_end) {
        let tag = arena[s];
        if (tag == 0u) {
            flush(&w);
            point(&w, apply(&w, bitcast<f32>(arena[s + 1u]), bitcast<f32>(arena[s + 2u])));
            s += 3u;
        } else if (tag == 1u) {
            if (w.count > 0u) {
                point(&w, apply(&w, bitcast<f32>(arena[s + 1u]), bitcast<f32>(arena[s + 2u])));
            }
            s += 3u;
        } else if (tag == 2u) {
            if (w.count > 0u) {
                let c1 = apply(&w, bitcast<f32>(arena[s + 1u]), bitcast<f32>(arena[s + 2u]));
                let c2 = apply(&w, bitcast<f32>(arena[s + 3u]), bitcast<f32>(arena[s + 4u]));
                let to = apply(&w, bitcast<f32>(arena[s + 5u]), bitcast<f32>(arena[s + 6u]));
                flatten_cubic(&w, w.prev_device, c1, c2, to);
            }
            s += 7u;
        } else {
            flush(&w);
            s += 1u;
        }
    }
    flush(&w);
    if (!EMIT) {
        counts[tile] = w.emitted;
    }
}
";

/// The deposit pass: ADR 0080's scanline shader, reading the v2 tile stride and the
/// scanned edge offsets. Its arithmetic is `raster/fill.rs` statement for statement;
/// the hazard notes live in `tests/compute_coverage_determinism.rs`, which measures
/// this port's reproducibility in isolation.
/// The exclusive prefix over the per-tile counts, computed where the counts are
/// (ADR 0095): one thread, tens of microseconds at fifty-eight thousand tiles, and
/// the host never reads a count again. Serial on purpose — a prefix sum has one
/// answer in integers, and one thread is the cheapest way to have exactly one order.
const SCAN: &str = r"
struct Params {
    stride_words: u32,
    rows: u32,
    tiles: u32,
    capacity: u32,
}

@group(0) @binding(0) var<storage, read> counts: array<u32>;
@group(0) @binding(1) var<storage, read_write> offsets: array<u32>;
@group(0) @binding(2) var<uniform> params: Params;

@compute @workgroup_size(1)
fn scan(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x != 0u) { return; }
    var total = 0u;
    for (var t = 0u; t < params.tiles; t += 1u) {
        offsets[t] = total;
        total += counts[t];
    }
    // The total rides at the end, where the emit's per-tile hard edge would read it
    // and where an eight-byte readback finds it beside the flag.
    offsets[params.tiles] = total;
}
";

const DEPOSIT: &str = r"
struct Params {
    stride_words: u32,
    rows: u32,
    tiles: u32,
    padding: u32,
}

@group(0) @binding(0) var<storage, read> tiles: array<u32>;
@group(0) @binding(1) var<storage, read> offsets: array<u32>;
@group(0) @binding(2) var<storage, read> counts: array<u32>;
@group(0) @binding(3) var<storage, read> edges: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> acc: array<f32>;
@group(0) @binding(5) var<storage, read_write> image: array<atomic<u32>>;
@group(0) @binding(6) var<uniform> params: Params;

const TILE_U32S: u32 = 20u;

// Which tile owns a row job: the records are in row_start order by construction, so
// the largest row_start at or below the job is a binary search — sixteen probes on a
// 58k-tile frame, against a host-built and host-uploaded job table (ADR 0083).
fn tile_of(job: u32) -> u32 {
    var lo = 0u;
    var hi = params.tiles;
    while (hi - lo > 1u) {
        let mid = (lo + hi) / 2u;
        if (tiles[mid * TILE_U32S + 8u] <= job) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    return lo;
}

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
    let tile = tile_of(job);
    let t = tile * TILE_U32S;
    let seat_x = tiles[t];
    let seat_y = tiles[t + 1u];
    let width = tiles[t + 2u];
    let height = tiles[t + 3u];
    let even_odd = tiles[t + 4u];
    let acc_start = tiles[t + 7u];
    let row_start = tiles[t + 8u];
    let edge_start = offsets[tile];
    let edge_count = counts[tile];
    let my_row = job - row_start;
    let fw = f32(width);
    let fh = f32(height);
    let fy = f32(my_row);
    let base = acc_start + my_row * (width + 1u);
    for (var e = 0u; e < edge_count; e += 1u) {
        let edge = edges[edge_start + e];
        let x0 = edge.x; let y0 = edge.y; let x1 = edge.z; let y1 = edge.w;
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
        if (abs(dxdy) > 3.0e38) { continue; }
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
            let a = abs(running);
            let m = a - 2.0 * floor(a * 0.5);
            cov = 1.0 - abs(m - 1.0);
        }
        let value = u32(floor(cov * 255.0 + 0.5));
        let byte = seat_x + x;
        atomicOr(&image[row_word + byte / 4u], value << (8u * (byte % 4u)));
    }
}
";

/// The lane's three pipelines, compiled together on the first frame that takes the
/// lane and kept — never on the startup path (§7).
#[derive(Debug)]
pub(crate) struct Pipelines {
    count: wgpu::ComputePipeline,
    /// The device-side prefix over the counts (ADR 0095).
    scan: wgpu::ComputePipeline,
    emit: wgpu::ComputePipeline,
    deposit: wgpu::ComputePipeline,
}

impl Pipelines {
    fn compile(gpu: &wgpu::Device) -> Self {
        let flatten_pipeline = |label: &str, emit: bool| {
            let source = format!("const EMIT: bool = {emit};\n{FLATTEN}");
            let module = gpu.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            gpu.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: None,
                module: &module,
                entry_point: Some("flatten_tiles"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        let deposit = gpu.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quorra compute deposit"),
            source: wgpu::ShaderSource::Wgsl(DEPOSIT.into()),
        });
        let scan = gpu.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quorra compute scan"),
            source: wgpu::ShaderSource::Wgsl(SCAN.into()),
        });
        Self {
            count: flatten_pipeline("quorra compute count", false),
            emit: flatten_pipeline("quorra compute emit", true),
            scan: gpu.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("quorra compute scan"),
                layout: None,
                module: &scan,
                entry_point: Some("scan"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            }),
            deposit: gpu.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("quorra compute deposit"),
                layout: None,
                module: &deposit,
                entry_point: Some("coverage_rows"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            }),
        }
    }
}

/// A `(1024, ⌈n / 65536⌉)` dispatch: the shaders index jobs as `y·65536 + x`, so no
/// dispatch dimension can overflow whatever `n` is.
fn grid(jobs: u32) -> (u32, u32) {
    (1024, jobs.div_ceil(65536).max(1))
}

/// The lane's two timestamp queries: the count pass, and the emit+deposit pair.
///
/// Kept for the device's life exactly as the frame's `PassQuery` is (ADR 0031). The
/// lane's dispatches run in submissions of their own before the content pass, so
/// without these its device time was invisible to the frame's one query — the
/// caller's ADR 0084 carried ~25 ms of "unattributed" per worst-page frame.
#[derive(Debug)]
pub(crate) struct ComputeQueries {
    /// Begin/end of the count dispatch.
    pub(crate) count: crate::timing::PassQuery,
    /// Begin of the emit dispatch, end of the deposit dispatch: one span for the
    /// submission that draws, because the two run back to back on one queue and the
    /// seam between them is not a number anybody acts on.
    pub(crate) coverage: crate::timing::PassQuery,
}

impl ComputeQueries {
    pub(crate) fn new(gpu: &wgpu::Device) -> Self {
        Self {
            count: crate::timing::PassQuery::new(gpu),
            coverage: crate::timing::PassQuery::new(gpu),
        }
    }
}

/// The chain in flight (ADR 0095): the eight bytes the host reads at the frame's
/// own end-wait — the overflow flag and the scanned total — and the capacity the
/// emit was guarded against, so the caller can grow and re-run before presenting.
#[derive(Debug)]
pub(crate) struct ComputeRun {
    readback: wgpu::Buffer,
}

impl ComputeRun {
    /// The flag and the total, read after the frame's own wait: `None` when the emit
    /// fit — the steady road — and `Some(total_edges)` where it met the capacity and
    /// the frame must grow and re-run before any present (ADR 0095).
    ///
    /// # Errors
    ///
    /// The map's own refusals; the work is already complete when this is called, so
    /// the poll returns without waiting.
    pub(crate) fn overflowed(&self, gpu: &wgpu::Device) -> Result<Option<u64>, RenderError> {
        let slice = self.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.poll(wgpu::PollType::wait_indefinitely())
            .map_err(|_| RenderError::ReadbackFailed {
                detail: "the device did not answer the overflow readback's map".into(),
            })?;
        let (flag, total) = {
            let mapped = slice
                .get_mapped_range()
                .map_err(|_| RenderError::ReadbackFailed {
                    detail: "the overflow readback did not map".into(),
                })?;
            let word = |at: usize| {
                mapped
                    .get(at..at.saturating_add(4))
                    .and_then(|bytes| bytes.try_into().ok())
                    .map_or(0, u32::from_le_bytes)
            };
            (word(0), word(4))
        };
        self.readback.unmap();
        Ok((flag != 0).then_some(u64::from(total)))
    }
}

/// The persistent half of the lane (ADR 0095): the edges buffer, grow-only, so a
/// steady zoom allocates nothing data-dependent and the host never waits mid-frame.
#[derive(Debug)]
pub(crate) struct ComputePersist {
    edges: Option<wgpu::Buffer>,
    /// The capacity in edges. Zero before the first frame; grown to the scanned
    /// total (plus headroom) whenever the flag says the emit met it.
    capacity_edges: u32,
}

impl ComputePersist {
    pub(crate) const fn new() -> Self {
        Self {
            edges: None,
            capacity_edges: 0,
        }
    }

    /// Grow to hold `total` edges, with a quarter of headroom so a slowly growing
    /// magnification does not re-run every frame, clamped to what `max_frame_bytes`
    /// prices. The refusal names both numbers, exactly as the per-frame allocation
    /// it replaces did.
    pub(crate) fn grow(
        &mut self,
        gpu: &wgpu::Device,
        total: u64,
        max_frame_bytes: u64,
    ) -> Result<(), RenderError> {
        let capacity = Self::price(total, max_frame_bytes)?;
        self.edges = Some(gpu.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quorra compute edges"),
            size: u64::from(capacity).saturating_mul(16).max(16),
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        }));
        self.capacity_edges = capacity;
        Ok(())
    }

    /// The growth's arithmetic, separated from its allocation so the refusal can be
    /// tested without a device — which also proves the check precedes the buffer.
    fn price(total: u64, max_frame_bytes: u64) -> Result<u32, RenderError> {
        let needed_bytes = total.saturating_mul(16);
        if needed_bytes > max_frame_bytes || total > u64::from(u32::MAX) {
            return Err(RenderError::FrameBudgetExceeded {
                needed: needed_bytes,
                budget: max_frame_bytes,
            });
        }
        let with_headroom = total
            .saturating_add(total / 4)
            .min(max_frame_bytes / 16)
            .max(total);
        #[allow(clippy::cast_possible_truncation)] // clamped to u32::MAX just above
        Ok(with_headroom.min(u64::from(u32::MAX)) as u32)
    }

    /// Bytes the persistent capacity holds, for the frame's staging accounting.
    pub(crate) fn capacity_bytes(&self) -> u64 {
        u64::from(self.capacity_edges).saturating_mul(16)
    }
}

/// Runs the whole lane in **one submission with no mid-frame readback** (ADR 0095):
/// residency, the exact count, the device-side scan of it, the guarded emit, the
/// deposit and the image round-trip — and schedules the eight-byte readback the
/// caller checks at the frame's own end-wait.
///
/// `Ok(None)` when no outline had segments: nothing to flatten, nothing to draw,
/// nothing to check.
///
/// # Errors
///
/// The arena's own upload refusals, and the growth refusal where even the starting
/// capacity cannot be priced.
#[allow(clippy::too_many_lines)]
// one frame's pass chain, in submission order — the stages read top to bottom and a
// split would scatter the ordering argument
#[allow(clippy::too_many_arguments)] // the device's own fields, threaded once from
// the one call site in `staging.rs`
#[allow(clippy::cast_possible_truncation)] // strides and word counts are bounded by
// the device's texture dimension and by the checks above each cast
pub(crate) fn dispatch_chain(
    gpu: &wgpu::Device,
    queue: &wgpu::Queue,
    pipelines: &mut Option<Pipelines>,
    arena: &mut SegmentArena,
    persist: &mut ComputePersist,
    resources: &crate::resources::ResourceStore,
    max_frame_bytes: u64,
    sheet: &wgpu::Texture,
    compute: &ComputeSheet,
    queries: Option<&ComputeQueries>,
    spans: &mut Vec<(&'static str, std::time::Duration)>,
) -> Result<Option<ComputeRun>, RenderError> {
    let pipelines = &*pipelines.get_or_insert_with(|| Pipelines::compile(gpu));
    // Residency first: every outline this frame draws, uploaded once ever.
    let residency_started = std::time::Instant::now();
    let mut records: Vec<u32> = Vec::with_capacity(compute.tiles.len().saturating_mul(TILE_U32S));
    for tile in &compute.tiles {
        let segments: &[Segment] = resources
            .outline(quorra_scene::OutlineId(tile.outline))
            .map_or(&[], |stored| &stored.segments);
        let (seg_offset, seg_len) = arena
            .ensure(gpu, queue, tile.outline, segments)
            .unwrap_or((0, 0));
        records.extend_from_slice(&[
            tile.seat[0],
            tile.seat[1],
            tile.width,
            tile.height,
            u32::from(tile.even_odd),
            seg_offset,
            seg_len,
            tile.acc_start,
            tile.row_start,
            0, // reserved
            tile.left.to_bits(),
            tile.top.to_bits(),
            tile.transform[0].to_bits(),
            tile.transform[1].to_bits(),
            tile.transform[2].to_bits(),
            tile.transform[3].to_bits(),
            tile.transform[4].to_bits(),
            tile.transform[5].to_bits(),
            0,
            0,
        ]);
    }
    spans.push(("compute residency+records", residency_started.elapsed()));
    let Some(arena_buffer) = arena.buffer.as_ref() else {
        return Ok(None); // no outline had segments: nothing to flatten, nothing to draw
    };
    if persist.capacity_edges == 0 {
        // The first frame's heuristic: enough for an ordinary page's tiles, priced by
        // the same clamp every growth is. A page it undershoots pays one re-run.
        let guess = (compute.tiles.len() as u64).saturating_mul(32).max(65_536);
        // The growth road's test seam: `tests/capacity_growth.rs` starts the capacity
        // at one edge so the first emit must overflow, and holds the re-run to the
        // steady road's bytes. Compiled out everywhere else.
        #[cfg(feature = "sabotage-capacity")]
        let guess = if std::env::var_os("QUORRA_SABOTAGE_CAPACITY").is_some() {
            1
        } else {
            guess
        };
        persist.grow(gpu, guess, max_frame_bytes)?;
    }
    let record_bytes: Vec<u8> = records.iter().flat_map(|word| word.to_le_bytes()).collect();
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
    let sized = |label: &str, bytes: u64, extra: wgpu::BufferUsages| {
        gpu.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes.max(16),
            usage: wgpu::BufferUsages::STORAGE | extra,
            mapped_at_creation: false,
        })
    };
    let tiles_buffer = storage("quorra compute tiles", &record_bytes);
    let tile_count = compute.tiles.len() as u32;
    let counts_buffer = sized(
        "quorra compute counts",
        u64::from(tile_count).saturating_mul(4),
        wgpu::BufferUsages::empty(),
    );
    // One extra entry holding the scanned total (ADR 0095): the emit's offsets, the
    // deposit's starts, and the readback's second word are all this one buffer.
    let offsets_buffer = sized(
        "quorra compute offsets",
        u64::from(tile_count).saturating_add(1).saturating_mul(4),
        wgpu::BufferUsages::COPY_SRC,
    );
    let stride = compute.stride() as u32;
    let params: Vec<u8> = [stride / 4, compute.rows, tile_count, persist.capacity_edges]
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect();
    let params_buffer = gpu.create_buffer(&wgpu::BufferDescriptor {
        label: Some("quorra compute params"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&params_buffer, 0, &params);
    let flag = sized(
        "quorra compute overflow flag",
        4,
        wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
    );
    queue.write_buffer(&flag, 0, &[0u8; 4]);
    let readback = gpu.create_buffer(&wgpu::BufferDescriptor {
        label: Some("quorra compute overflow readback"),
        size: 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    // The count pass binds the offsets and edges slots to placeholders: the shader
    // touches neither under EMIT = false, and a bind group needs a buffer either way
    // — two of them, because one buffer bound read-only and read-write in a single
    // dispatch is a usage conflict wgpu refuses. The flag's placeholder exists
    // because the layout keeps a const-dead binding.
    let placeholder_read = sized(
        "quorra compute placeholder",
        16,
        wgpu::BufferUsages::empty(),
    );
    let placeholder_write = sized(
        "quorra compute placeholder",
        16,
        wgpu::BufferUsages::empty(),
    );
    let placeholder_flag = sized("quorra compute placeholder", 4, wgpu::BufferUsages::empty());
    let acc_buffer = sized(
        "quorra compute accumulator",
        compute.acc_floats.saturating_mul(4),
        wgpu::BufferUsages::empty(),
    );
    let image = sized(
        "quorra compute image",
        u64::from(stride).saturating_mul(u64::from(compute.height)),
        wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
    );
    #[allow(clippy::expect_used)] // set by the grow() call above on every road here
    let edges_buffer = persist
        .edges
        .as_ref()
        .expect("grown before any dispatch")
        .clone();

    let flatten_group = |pipeline: &wgpu::ComputePipeline,
                         offsets: &wgpu::Buffer,
                         edges: &wgpu::Buffer,
                         flag_slot: &wgpu::Buffer| {
        gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra compute flatten"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                entry(0, arena_buffer),
                entry(1, &tiles_buffer),
                entry(2, &counts_buffer),
                entry(3, offsets),
                entry(4, edges),
                entry(5, &params_buffer),
                entry(6, flag_slot),
            ],
        })
    };
    let count_group = flatten_group(
        &pipelines.count,
        &placeholder_read,
        &placeholder_write,
        &placeholder_flag,
    );
    let emit_group = flatten_group(&pipelines.emit, &offsets_buffer, &edges_buffer, &flag);
    let scan_group = gpu.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("quorra compute scan"),
        layout: &pipelines.scan.get_bind_group_layout(0),
        entries: &[
            entry(0, &counts_buffer),
            entry(1, &offsets_buffer),
            entry(2, &params_buffer),
        ],
    });
    let deposit_group = gpu.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("quorra compute deposit"),
        layout: &pipelines.deposit.get_bind_group_layout(0),
        entries: &[
            entry(0, &tiles_buffer),
            entry(1, &offsets_buffer),
            entry(2, &counts_buffer),
            entry(3, &edges_buffer),
            entry(4, &acc_buffer),
            entry(5, &image),
            entry(6, &params_buffer),
        ],
    });

    // The whole chain, one submission, in pass order: count, scan, emit, deposit,
    // the image round-trip, and the eight bytes the end of the frame will read.
    let mut encoder = gpu.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("quorra compute chain"),
    });
    let extent = wgpu::Extent3d {
        width: compute.width,
        height: compute.height,
        depth_or_array_layers: 1,
    };
    let sheet_copy = wgpu::TexelCopyTextureInfo {
        texture: sheet,
        mip_level: 0,
        origin: wgpu::Origin3d::ZERO,
        aspect: wgpu::TextureAspect::All,
    };
    let layout = wgpu::TexelCopyBufferLayout {
        offset: 0,
        bytes_per_row: Some(stride),
        rows_per_image: None,
    };
    encoder.copy_texture_to_buffer(
        sheet_copy,
        wgpu::TexelCopyBufferInfo {
            buffer: &image,
            layout,
        },
        extent,
    );
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: None,
            timestamp_writes: queries.map(|q| wgpu::ComputePassTimestampWrites {
                query_set: &q.count.set,
                beginning_of_pass_write_index: Some(0),
                end_of_pass_write_index: Some(1),
            }),
        });
        pass.set_pipeline(&pipelines.count);
        pass.set_bind_group(0, &count_group, &[]);
        let (x, y) = grid(tile_count);
        pass.dispatch_workgroups(x, y, 1);
    }
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&pipelines.scan);
        pass.set_bind_group(0, &scan_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: None,
            timestamp_writes: queries.map(|q| wgpu::ComputePassTimestampWrites {
                query_set: &q.coverage.set,
                beginning_of_pass_write_index: Some(0),
                end_of_pass_write_index: None,
            }),
        });
        pass.set_pipeline(&pipelines.emit);
        pass.set_bind_group(0, &emit_group, &[]);
        let (x, y) = grid(tile_count);
        pass.dispatch_workgroups(x, y, 1);
    }
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: None,
            timestamp_writes: queries.map(|q| wgpu::ComputePassTimestampWrites {
                query_set: &q.coverage.set,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: Some(1),
            }),
        });
        pass.set_pipeline(&pipelines.deposit);
        pass.set_bind_group(0, &deposit_group, &[]);
        let (x, y) = grid(compute.rows);
        pass.dispatch_workgroups(x, y, 1);
    }
    encoder.copy_buffer_to_texture(
        wgpu::TexelCopyBufferInfo {
            buffer: &image,
            layout,
        },
        sheet_copy,
        extent,
    );
    encoder.copy_buffer_to_buffer(&flag, 0, &readback, 0, 4);
    encoder.copy_buffer_to_buffer(
        &offsets_buffer,
        u64::from(tile_count).saturating_mul(4),
        &readback,
        4,
        4,
    );
    if let Some(q) = queries {
        encoder.resolve_query_set(&q.count.set, 0..2, &q.count.resolve, 0);
        encoder.copy_buffer_to_buffer(&q.count.resolve, 0, &q.count.map, 0, 16);
        encoder.resolve_query_set(&q.coverage.set, 0..2, &q.coverage.resolve, 0);
        encoder.copy_buffer_to_buffer(&q.coverage.resolve, 0, &q.coverage.map, 0, 16);
    }
    queue.submit([encoder.finish()]);
    Ok(Some(ComputeRun { readback }))
}

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

#[cfg(test)]
mod persist_tests {
    use super::ComputePersist;
    use crate::error::RenderError;

    /// The growth refusal keeps the per-frame allocation's name and both numbers
    /// (ADR 0095), and it precedes any buffer — provable here exactly because
    /// `price` needs no device to refuse.
    #[test]
    fn growth_past_the_budget_refuses_by_name() {
        // 2^28 edges is 4 GiB of edge bytes against a 1 MiB budget.
        let refused = ComputePersist::price(1 << 28, 1 << 20);
        match refused {
            Err(RenderError::FrameBudgetExceeded { needed, budget }) => {
                assert_eq!(needed, (1_u64 << 28) * 16);
                assert_eq!(budget, 1 << 20);
            }
            other => panic!("a total past the budget refuses by name: {other:?}"),
        }
        // Inside the budget: a quarter of headroom, clamped by what the budget
        // prices.
        let priced = ComputePersist::price(1_000, u64::MAX).expect("fits");
        assert_eq!(priced, 1_250);
    }
}

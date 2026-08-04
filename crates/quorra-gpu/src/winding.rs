//! The GPU coverage lane's frame work: two passes, once per group of four samples.
//!
//! The encoder hands over a [`Sheet`] — outline triangles in sheet space, and the
//! tiles they belong to. This module turns that into the same R8 scratch texture the
//! CPU lane uploads, so that everything downstream of coverage (the quad lanes, clips,
//! knockout, the compositor) cannot tell which lane produced it. That is the whole
//! integration: one texture, two producers, no second code path (ADR 0016).
//!
//! # Why a group of four
//!
//! An `rgba16float` texel holds four signed winding numbers exactly, so four sample
//! positions cost one texel and no packing. A frame that wants sixteen samples runs
//! this pair of passes four times, clearing the winding texture between rounds and
//! adding each round's quarter into the sheet — so **sample count costs time, never
//! memory**, which is the trade a document renderer wants: the GPU is idle here
//! (`execute` is tens of microseconds) and memory is what a zoomed page runs out of.

// Dead until the encoder routes fills here (ADR 0016's "what is landed"): every item
// below is reached only by this module's own tests today. `allow` rather than removal,
// with the condition for its going away named: it goes when `encode.rs` gains the
// coverage selector, and the same commit that adds the caller deletes this line.
#![allow(dead_code)]

use crate::error::RenderError;
use crate::pipeline::{Kind, PipelineStore};

/// Bytes one resolve instance occupies: the tile rectangle and its fill rule.
pub(crate) const TILE_STRIDE: u64 = 24;

/// The winding texture's format. `f16` is exact on integers to 2048, which bounds the
/// winding number this lane can represent — four hundred times any winding a real page
/// produces, and stated here because the format is where the bound comes from.
const WINDING_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Samples per pass: one per channel of the winding texel.
pub(crate) const SAMPLES_PER_PASS: u32 = 4;

/// One tile of coverage the resolve pass turns into bytes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tile {
    /// The tile in scratch pixels: min x, min y, max x, max y.
    pub rect: [f32; 4],
    /// ISO 32000-2 §8.5.3.3's rule: `false` non-zero, `true` even-odd.
    pub even_odd: bool,
}

/// What the encoder built for the GPU lane this frame.
#[derive(Debug, Default)]
pub(crate) struct Sheet {
    /// Every triangle of every tile, in sheet space, as the vertex buffer's floats.
    pub vertices: Vec<f32>,
    /// One entry per packed tile.
    pub tiles: Vec<Tile>,
    /// The scratch sheet's size in pixels.
    pub width: u32,
    pub height: u32,
}

impl Sheet {
    /// Whether this frame drew anything through the GPU lane.
    pub(crate) fn is_empty(&self) -> bool {
        self.tiles.is_empty() || self.vertices.is_empty()
    }

    /// Bytes this sheet costs on the device, for the frame budget: the winding texture
    /// plus the vertex and instance buffers. Counted before anything is allocated,
    /// because a buffer sized from document-derived arithmetic is exactly what
    /// principle 3 says to check first.
    #[allow(clippy::cast_possible_truncation)] // lengths of Vecs this frame just built
    pub(crate) fn device_bytes(&self) -> u64 {
        // Saturating throughout: the number this returns is *checked against* a budget,
        // so a sheet too large to size must come back too large rather than wrap to
        // something affordable. That is principle 3's rule about allocations derived
        // from scene content, applied to the arithmetic that describes them.
        let texels = u64::from(self.width).saturating_mul(u64::from(self.height));
        let winding = texels.saturating_mul(8); // rgba16float
        let vertices = (self.vertices.len() as u64).saturating_mul(4);
        let tiles = (self.tiles.len() as u64).saturating_mul(TILE_STRIDE);
        winding.saturating_add(vertices).saturating_add(tiles)
    }
}

/// The ordered sample grid, in pixels relative to the pixel's centre.
///
/// `count` samples on an `n × n` grid, `n = √count`, the k-th at
/// `((k mod n) + ½)/n − ½` across and `((k / n) + ½)/n − ½` down. Ours rather than the
/// driver's, so two adapters place them identically (ADR 0006's promise), and ordered
/// rather than jittered so that a frame is reproducible without carrying a seed.
#[allow(clippy::arithmetic_side_effects)] // `side` is at least 1 and at most 16
#[allow(clippy::cast_precision_loss)] // grid indices, far below f32's exact range
pub(crate) fn sample_offsets(count: u32) -> Vec<[f32; 2]> {
    let side = count.isqrt().max(1);
    #[allow(clippy::cast_precision_loss)] // side is at most 16 by Options' validation
    let step = 1.0 / side as f32;
    (0..count)
        .map(|index| {
            #[allow(clippy::cast_precision_loss)]
            let (x, y) = ((index % side) as f32, (index / side) as f32);
            [(x + 0.5).mul_add(step, -0.5), (y + 0.5).mul_add(step, -0.5)]
        })
        .collect()
}

/// Renders `sheet` into a fresh R8 coverage texture.
///
/// # Errors
///
/// [`RenderError::TargetTooLarge`] when the packed sheet exceeds the adapter's texture
/// dimension — the same limit, named the same way, as any other target of ours.
pub(crate) fn render(
    gpu: &wgpu::Device,
    queue: &wgpu::Queue,
    pipelines: &PipelineStore,
    sheet: &Sheet,
    samples: u32,
    max_dimension: u32,
) -> Result<(wgpu::Texture, wgpu::TextureView), RenderError> {
    if sheet.width > max_dimension || sheet.height > max_dimension {
        return Err(RenderError::TargetTooLarge {
            width: sheet.width,
            height: sheet.height,
            limit: max_dimension,
        });
    }
    let extent = wgpu::Extent3d {
        width: sheet.width.max(1),
        height: sheet.height.max(1),
        depth_or_array_layers: 1,
    };
    let coverage = gpu.create_texture(&wgpu::TextureDescriptor {
        label: Some("quorra scratch coverage (gpu lane)"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        // COPY_SRC as well as the two it needs: the coverage sheet is the one artefact
        // that decides whether this lane agrees with the CPU one, and a sheet nothing
        // can read back is a lane nothing can hold to a bound.
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let coverage_view = coverage.create_view(&wgpu::TextureViewDescriptor::default());
    let winding = gpu.create_texture(&wgpu::TextureDescriptor {
        label: Some("quorra winding"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: WINDING_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let winding_view = winding.create_view(&wgpu::TextureViewDescriptor::default());

    let buffers = Buffers::new(gpu, queue, pipelines, sheet, samples);
    let (winding_pipeline, _) = pipelines.get(Kind::Winding, WINDING_FORMAT);
    let (resolve_pipeline, _) = pipelines.get(Kind::WindingResolve, wgpu::TextureFormat::R8Unorm);
    let texture_layout = pipelines.blit_layout();
    let winding_source = gpu.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("quorra winding source"),
        layout: &texture_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&winding_view),
        }],
    });

    let mut encoder = gpu.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("quorra coverage"),
    });
    for (round, group) in buffers.groups.iter().enumerate() {
        accumulate(
            &mut encoder,
            &winding_view,
            &winding_pipeline,
            &buffers,
            group,
        );
        resolve(
            &mut encoder,
            &coverage_view,
            &resolve_pipeline,
            &buffers,
            group,
            &winding_source,
            round == 0,
        );
    }
    queue.submit([encoder.finish()]);
    Ok((coverage, coverage_view))
}

/// One round's winding pass: clear, then one draw per sample of the group.
fn accumulate(
    encoder: &mut wgpu::CommandEncoder,
    winding_view: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    buffers: &Buffers,
    group: &Group,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("quorra winding"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: winding_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                // Every round starts from no winding at all: the sheet accumulates
                // coverage, the winding texture does not accumulate across rounds.
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_vertex_buffer(0, buffers.vertices.slice(..));
    for bind_group in &group.samples {
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..buffers.vertex_count, 0..1);
    }
}

/// One round's resolve: each tile's quad turns four samples into a quarter of its
/// coverage, added to whatever earlier rounds contributed.
fn resolve(
    encoder: &mut wgpu::CommandEncoder,
    coverage_view: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    buffers: &Buffers,
    group: &Group,
    winding_source: &wgpu::BindGroup,
    first: bool,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("quorra winding resolve"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: coverage_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                // Cleared once, then added to: a tile the encoder packed but no round
                // covers is transparent, which is the truth about it.
                load: if first {
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                } else {
                    wgpu::LoadOp::Load
                },
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &group.resolve, &[]);
    pass.set_bind_group(1, winding_source, &[]);
    pass.set_vertex_buffer(0, buffers.tiles.slice(..));
    pass.draw(0..4, 0..buffers.tile_count);
}

/// One group of four samples: a bind group per sample for the winding pass, and one
/// for the resolve that follows it.
struct Group {
    samples: Vec<wgpu::BindGroup>,
    resolve: wgpu::BindGroup,
}

/// Everything the two passes read, built once per frame.
struct Buffers {
    vertices: wgpu::Buffer,
    vertex_count: u32,
    tiles: wgpu::Buffer,
    tile_count: u32,
    groups: Vec<Group>,
}

impl Buffers {
    #[allow(clippy::cast_precision_loss)] // sheet extents are far below f32's exact range
    #[allow(clippy::cast_possible_truncation)] // a frame with 2^32 vertices was refused
    // by the budget long before it reached this cast
    #[allow(clippy::arithmetic_side_effects)] // a Vec length times its element count
    fn new(
        gpu: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &PipelineStore,
        sheet: &Sheet,
        samples: u32,
    ) -> Self {
        let vertices = create_buffer(
            gpu,
            queue,
            "quorra winding vertices",
            &to_bytes(&sheet.vertices),
            wgpu::BufferUsages::VERTEX,
        );
        let mut tile_data: Vec<f32> = Vec::with_capacity(sheet.tiles.len() * 6);
        for tile in &sheet.tiles {
            tile_data.extend_from_slice(&tile.rect);
            tile_data.push(if tile.even_odd { 1.0 } else { 0.0 });
            tile_data.push(samples as f32);
        }
        let tiles = create_buffer(
            gpu,
            queue,
            "quorra winding tiles",
            &to_bytes(&tile_data),
            wgpu::BufferUsages::VERTEX,
        );

        let layout = pipelines.winding_layout();
        let offsets = sample_offsets(samples);
        let groups = offsets
            .chunks(SAMPLES_PER_PASS as usize)
            .map(|chunk| {
                let samples = chunk
                    .iter()
                    .enumerate()
                    .map(|(channel, offset)| {
                        globals_bind_group(gpu, queue, &layout, sheet, *offset, channel)
                    })
                    .collect();
                // The resolve reads the sheet size only; the offset and channel are
                // the winding pass's business, so any of the group's values will do.
                let resolve = globals_bind_group(gpu, queue, &layout, sheet, [0.0, 0.0], 0);
                Group { samples, resolve }
            })
            .collect();

        Self {
            vertex_count: (sheet.vertices.len() / 8) as u32,
            vertices,
            tile_count: sheet.tiles.len() as u32,
            tiles,
            groups,
        }
    }
}

/// The 32-byte uniform one winding draw reads.
#[allow(clippy::cast_precision_loss)] // sheet extents are far below f32's exact range
fn globals_bind_group(
    gpu: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sheet: &Sheet,
    offset: [f32; 2],
    channel: usize,
) -> wgpu::BindGroup {
    let mut mask = [0.0_f32; 4];
    mask[channel.min(3)] = 1.0;
    let data = [
        sheet.width.max(1) as f32,
        sheet.height.max(1) as f32,
        offset[0],
        offset[1],
        mask[0],
        mask[1],
        mask[2],
        mask[3],
    ];
    let buffer = create_buffer(
        gpu,
        queue,
        "quorra winding globals",
        &to_bytes(&data),
        wgpu::BufferUsages::UNIFORM,
    );
    gpu.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("quorra winding globals"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    })
}

/// A buffer with `data` in it. Zero-length data still makes a one-element buffer:
/// wgpu refuses a zero-sized buffer, and a frame with no triangles is a legitimate
/// frame that must still reach the passes and draw nothing.
fn create_buffer(
    gpu: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    data: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    let size = (data.len() as u64).max(4);
    let buffer = gpu.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if !data.is_empty() {
        queue.write_buffer(&buffer, 0, data);
    }
    buffer
}

/// Floats as the little-endian bytes a wgpu buffer takes.
///
/// A copy rather than a cast: `#![forbid(unsafe_code)]` rules out reinterpreting the
/// slice, and the crate takes no `bytemuck` (`deny.toml`'s posture is that a
/// dependency has to earn its place). The copy is one `memcpy` per frame over data
/// the encoder has just built anyway, which no measurement has ever noticed.
fn to_bytes(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(size_of_val(values));
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)] // test-file policy as in `raster.rs`: a fixture that cannot run must fail loudly
mod tests {
    use super::{Sheet, Tile, render, sample_offsets};
    use crate::device::Device;
    use crate::outline::QuadOutline;
    use crate::startup::Options;
    use quorra_scene::{Point, Segment};

    /// The software adapter, as everywhere in this crate's tests.
    fn device() -> Device {
        Device::headless(&Options {
            adapter: Some("llvmpipe".into()),
            ..Options::default()
        })
        .expect("llvmpipe is present wherever this suite runs")
    }

    const SIDE: u32 = 16;

    /// The R8 coverage bytes of a sheet rendered from these segments.
    fn coverage(segments: &[Segment], even_odd: bool, samples: u32) -> Vec<u8> {
        let device = device();
        device.wait_until_warm();
        let (gpu, queue) = device.wgpu();
        let outline = QuadOutline::from_segments(segments, None);
        let mut vertices = Vec::new();
        let mut floats = Vec::new();
        outline.append_triangles(
            |p| [p.x, p.y],
            [0.0, 0.0, SIDE as f32, SIDE as f32],
            &mut vertices,
        );
        for vertex in &vertices {
            floats.extend_from_slice(&vertex.floats());
        }
        let sheet = Sheet {
            vertices: floats,
            tiles: vec![Tile {
                rect: [0.0, 0.0, SIDE as f32, SIDE as f32],
                even_odd,
            }],
            width: SIDE,
            height: SIDE,
        };
        let (texture, _) = render(
            gpu,
            queue,
            device.pipeline_store(),
            &sheet,
            samples,
            device.limits().max_target_size,
        )
        .expect("the sheet is inside every limit");

        // Copy out, 256-aligned as wgpu requires, and drop the padding.
        let row = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = SIDE.next_multiple_of(row);
        let buffer = gpu.create_buffer(&wgpu::BufferDescriptor {
            label: Some("winding test readback"),
            size: u64::from(padded) * u64::from(SIDE),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder =
            gpu.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.poll(wgpu::PollType::wait_indefinitely())
            .expect("the copy completes");
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .expect("the buffer is mapped");
        let mut pixels = Vec::with_capacity((SIDE * SIDE) as usize);
        for y in 0..SIDE as usize {
            let start = y * padded as usize;
            pixels.extend_from_slice(&mapped[start..start + SIDE as usize]);
        }
        drop(mapped);
        buffer.unmap();
        pixels
    }

    fn at(pixels: &[u8], x: u32, y: u32) -> u8 {
        pixels[(y * SIDE + x) as usize]
    }

    fn rect_path(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<Segment> {
        vec![
            Segment::MoveTo(Point::new(x0, y0)),
            Segment::LineTo(Point::new(x1, y0)),
            Segment::LineTo(Point::new(x1, y1)),
            Segment::LineTo(Point::new(x0, y1)),
            Segment::Close,
        ]
    }

    /// A pixel-aligned square is solid inside and empty outside — the coarsest thing
    /// the lane must get right, and the one that catches a sign or winding error.
    #[test]
    fn an_aligned_square_is_solid_inside_and_empty_outside() {
        let pixels = coverage(&rect_path(4.0, 4.0, 12.0, 12.0), false, 16);
        assert_eq!(at(&pixels, 8, 8), 255, "the middle is covered");
        assert_eq!(at(&pixels, 4, 4), 255, "the first covered pixel");
        assert_eq!(at(&pixels, 11, 11), 255, "the last covered pixel");
        assert_eq!(at(&pixels, 3, 8), 0, "one pixel left of the square");
        assert_eq!(at(&pixels, 12, 8), 0, "one pixel right of the square");
        assert_eq!(at(&pixels, 0, 0), 0, "the corner nothing reaches");
    }

    /// An edge through the middle of a pixel column covers exactly half of it, and the
    /// sample grid says so exactly: with a 4×4 grid the columns sit at ±0.125 and
    /// ±0.375 from the centre, so two of four are inside — 8 of 16 samples, and
    /// `round(0.5 × 255)` is 128.
    #[test]
    fn a_half_covered_column_reads_one_hundred_and_twenty_eight() {
        let pixels = coverage(&rect_path(4.5, 4.0, 12.0, 12.0), false, 16);
        assert_eq!(at(&pixels, 4, 8), 128, "half of column 4 is inside");
        assert_eq!(at(&pixels, 5, 8), 255, "column 5 is wholly inside");
        assert_eq!(at(&pixels, 3, 8), 0, "column 3 is wholly outside");
    }

    /// Two nested squares wound the same way: non-zero fills the hole (winding 2),
    /// even-odd leaves it (winding 2 is even). The two rules differing on the *same*
    /// geometry is what proves the sign survived accumulation — a lane that only ever
    /// counted crossings could not tell these apart.
    #[test]
    fn the_two_fill_rules_differ_where_the_clause_says_they_do() {
        let mut nested = rect_path(2.0, 2.0, 14.0, 14.0);
        nested.extend(rect_path(6.0, 6.0, 10.0, 10.0));
        let non_zero = coverage(&nested, false, 16);
        let even_odd = coverage(&nested, true, 16);
        assert_eq!(
            at(&non_zero, 8, 8),
            255,
            "§8.5.3.3.2: winding two is not zero, so the inner square is filled"
        );
        assert_eq!(
            at(&even_odd, 8, 8),
            0,
            "§8.5.3.3.3: winding two is even, so the inner square is a hole"
        );
        assert_eq!(at(&non_zero, 3, 8), 255, "the outer ring fills either way");
        assert_eq!(at(&even_odd, 3, 8), 255);
    }

    /// The sample grid is an ordered grid, stated rather than the driver's: the k-th of
    /// sixteen sits at a quarter-pixel step, symmetric about the centre.
    #[test]
    fn the_sample_grid_is_ordered_and_centred() {
        let offsets = sample_offsets(16);
        assert_eq!(offsets.len(), 16);
        assert!((offsets[0][0] + 0.375).abs() < 1e-6, "{:?}", offsets[0]);
        assert!((offsets[15][1] - 0.375).abs() < 1e-6, "{:?}", offsets[15]);
        let sum: f32 = offsets.iter().map(|o| o[0] + o[1]).sum();
        assert!(
            sum.abs() < 1e-5,
            "the grid is balanced about the centre: {sum}"
        );
    }
}

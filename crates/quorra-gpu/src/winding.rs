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

use crate::error::RenderError;
use crate::outline::WindingVertex;
use crate::pane::{Pane, Plan, TILE_STRIDE, Tile, vertex_floats};
use crate::pipeline::{Kind, PipelineStore};

/// The winding texture's format. `f16` is exact on integers to 2048, which bounds the
/// winding number this lane can represent — four hundred times any winding a real page
/// produces, and stated here because the format is where the bound comes from.
const WINDING_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Samples per pass: one per channel of the winding texel.
pub(crate) const SAMPLES_PER_PASS: u32 = 4;

/// What the encoder built for the GPU lane this frame.
#[derive(Debug, Default)]
pub(crate) struct Sheet {
    /// Every triangle of every tile, in sheet space, as the vertex buffer's floats.
    pub vertices: Vec<f32>,
    /// One entry per packed tile, each owning a run of `vertices`.
    pub tiles: Vec<Tile>,
    /// The scratch sheet's size in pixels.
    pub width: u32,
    pub height: u32,
}

impl Sheet {
    /// Adds a tile and the triangles that fill it.
    ///
    /// The tile records where its vertices went, which is what lets a pane draw its own
    /// tiles' triangles and nobody else's (`crate::pane`).
    pub(crate) fn push_tile(&mut self, rect: [f32; 4], even_odd: bool, vertices: &[WindingVertex]) {
        // A tile with no triangles is not a tile: it would resolve to transparent,
        // which the sheet already is there.
        if vertices.is_empty() {
            return;
        }
        // A frame with 2^32 vertices was refused by the byte budget long before it
        // reached this cast, and saturating keeps the ranges monotonic if one ever does.
        let first_vertex =
            u32::try_from(self.vertices.len() / WindingVertex::FLOATS).unwrap_or(u32::MAX);
        vertex_floats(vertices, &mut self.vertices);
        self.tiles.push(Tile {
            rect,
            even_odd,
            first_vertex,
            vertex_count: u32::try_from(vertices.len()).unwrap_or(u32::MAX),
        });
    }

    /// How this frame's tiles are cut into what the winding target holds at once.
    pub(crate) fn plan(&self) -> Plan {
        Plan::new(&self.tiles, [self.width, self.height])
    }

    /// Whether this frame drew anything through the GPU lane.
    pub(crate) fn is_empty(&self) -> bool {
        self.tiles.is_empty() || self.vertices.is_empty()
    }

    /// Bytes this sheet costs on the device, for the frame budget: the winding texture
    /// plus the vertex and instance buffers. Counted before anything is allocated,
    /// because a buffer sized from document-derived arithmetic is exactly what
    /// principle 3 says to check first.
    ///
    /// **A sheet with no tiles costs nothing, whatever extent it carries**, and that
    /// condition lives here rather than at either caller. The extent is the *scratch*
    /// sheet's, which both lanes share: `width` and `height` are filled in from it on
    /// every frame that packs a tile, including one the GPU lane never ran. Pricing
    /// the winding texture from that extent charges a CPU-lane frame for a texture
    /// [`render_into`] is never asked to make — the frame is refused for bytes nobody
    /// would have allocated, which is principle 6's failure with the sign flipped:
    /// a page that draws, refused. Five real corpus pages were, at up to 1.2 GB
    /// claimed against a 256 MiB budget for an empty sheet 16 384 texels wide.
    #[allow(clippy::cast_possible_truncation)] // lengths of Vecs this frame just built
    pub(crate) fn device_bytes(&self) -> u64 {
        // Not merely an optimisation of the arithmetic below: `is_empty` is exactly the
        // condition `Device::upload_scratch` allocates under, and saying it once is what
        // stops the pre-flight and the allocation from disagreeing again.
        if self.is_empty() {
            return 0;
        }
        // Saturating throughout: the number this returns is *checked against* a budget,
        // so a sheet too large to size must come back too large rather than wrap to
        // something affordable. That is principle 3's rule about allocations derived
        // from scene content, applied to the arithmetic that describes them.
        // The winding target holds one *pane*, not the sheet (ADR 0028).
        let winding = self.plan().target_bytes();
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

/// Draws `sheet`'s tiles into `coverage_view`, which is the frame's scratch sheet.
///
/// `clear` says whether this pass owns the whole texture: false when the CPU lane
/// uploaded bytes into the same sheet, and then the tiles drawn here land beside those
/// bytes without touching them — each tile's quad covers only its own rectangle.
///
/// # Errors
///
/// [`RenderError::TargetTooLarge`] when the packed sheet exceeds the adapter's texture
/// dimension — the same limit, named the same way, as any other target of ours.
#[allow(clippy::too_many_arguments)] // the pass's inputs, named once at the one call
pub(crate) fn render_into(
    gpu: &wgpu::Device,
    queue: &wgpu::Queue,
    pipelines: &PipelineStore,
    reuse: &mut WindingTexture,
    coverage_view: &wgpu::TextureView,
    sheet: &Sheet,
    samples: u32,
    clear: bool,
    max_dimension: u32,
) -> Result<(), RenderError> {
    if sheet.width > max_dimension || sheet.height > max_dimension {
        return Err(RenderError::TargetTooLarge {
            width: sheet.width,
            height: sheet.height,
            limit: max_dimension,
        });
    }
    // One pane at a time (ADR 0028): the target is scratch, so it is sized by what a
    // pass holds — a rectangle over this lane's own tiles — rather than by what the
    // page came to on a sheet both lanes share.
    let plan = sheet.plan();
    let extent = wgpu::Extent3d {
        width: plan.target[0].max(1),
        height: plan.target[1].max(1),
        depth_or_array_layers: 1,
    };
    // **Kept between frames**, which ADR 0012 declined to do for the compositor's
    // textures "until a measurement says otherwise". This is that measurement: at 20x
    // the sheet is 2.5 million texels, and allocating and zero-initialising eight
    // bytes of each, every frame, cost 10.7 ms of a 15 ms frame — more than the
    // rasterising the lane exists to avoid. One texture, grown when a frame needs a
    // larger one and never shrunk while the device lives.
    let winding_view = reuse.view_for(gpu, extent).clone();

    let buffers = Buffers::new(gpu, queue, sheet, &plan, samples);
    let (winding_pipeline, _) = pipelines.get(Kind::Winding, WINDING_FORMAT);
    let (resolve_pipeline, _) = pipelines.get(Kind::WindingResolve, wgpu::TextureFormat::R8Unorm);
    let texture_layout = pipelines.sampled_layout();
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
    for (index, pane) in plan.panes.iter().enumerate() {
        for (round, group) in buffers.groups.iter().enumerate() {
            let globals = group.for_pane(gpu, queue, pipelines, sheet, pane);
            accumulate(
                &mut encoder,
                &winding_view,
                &winding_pipeline,
                &buffers,
                &globals,
                pane,
            );
            resolve(
                &mut encoder,
                coverage_view,
                &resolve_pipeline,
                &buffers,
                &globals,
                pane,
                &winding_source,
                // The coverage sheet is cleared once, by the first pass that touches
                // it, and only when this lane owns it: every later pane and round adds
                // to what is there.
                clear && index == 0 && round == 0,
            );
        }
    }
    queue.submit([encoder.finish()]);
    Ok(())
}

/// The winding target, kept across frames.
///
/// Not a pool: one texture, because a frame has one sheet. It grows to the largest
/// extent any frame has needed — a viewer that zooms in and out repeatedly should not
/// pay an allocation each time it crosses a size it has already seen — and the bytes
/// are still charged to every frame that uses them, because what the frame *needs* is
/// what a budget is about, not what happens to be resident.
///
/// **The pane being drawn is the top-left of it**, whatever the rest of it is. Growing
/// and never shrinking is what makes that a thing to state rather than a tautology: a
/// smaller frame after a larger one gets a texture with room to spare, and `fs_resolve`
/// reads the pane's texels out of this texture's top-left corner. [`accumulate`]'s
/// viewport is what puts them there; `tests/frame_independence.rs` is what keeps them
/// there.
#[derive(Debug, Default)]
pub(crate) struct WindingTexture {
    held: Option<(wgpu::Extent3d, wgpu::Texture, wgpu::TextureView)>,
}

impl WindingTexture {
    /// A view of a texture at least `extent` in both dimensions.
    fn view_for(&mut self, gpu: &wgpu::Device, extent: wgpu::Extent3d) -> &wgpu::TextureView {
        let fits = self
            .held
            .as_ref()
            .is_some_and(|(held, _, _)| held.width >= extent.width && held.height >= extent.height);
        if !fits {
            let size = wgpu::Extent3d {
                width: self
                    .held
                    .as_ref()
                    .map_or(extent.width, |(held, _, _)| held.width.max(extent.width)),
                height: self
                    .held
                    .as_ref()
                    .map_or(extent.height, |(held, _, _)| held.height.max(extent.height)),
                depth_or_array_layers: 1,
            };
            let texture = gpu.create_texture(&wgpu::TextureDescriptor {
                label: Some("quorra winding"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: WINDING_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.held = Some((size, texture, view));
        }
        // `held` is `Some` on both paths — it either fitted or was just replaced — and
        // saying so with a match rather than an `expect` keeps the invariant in the
        // type rather than in a panic message.
        match self.held.as_ref() {
            Some((_, _, view)) => view,
            None => unreachable!("the branch above assigns `held` when it does not fit"),
        }
    }
}

/// One round's winding pass for one pane: clear, then one draw per sample of the group.
///
/// The pane's extent is **not** the attachment's size: [`WindingTexture`] is kept between
/// frames and is at least as large as any pane it has held, and the frame's own panes
/// differ in size from one another. See the viewport below for what that costs if it is
/// forgotten.
#[allow(clippy::cast_precision_loss)] // an extent bounded by the adapter's texture limit
fn accumulate(
    encoder: &mut wgpu::CommandEncoder,
    winding_view: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    buffers: &Buffers,
    globals: &PaneGlobals,
    pane: &Pane,
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
    // **The pane is the top-left of this texture, not the whole of it.** `vs_winding`
    // divides by the *pane's* size to reach clip space, and clip space spans whatever is
    // attached — so with a texture kept from a larger frame, and no viewport, every
    // pane pixel would be written `held / pane` times further down and across than the
    // resolve pass reads it. The viewport is what makes the two agree, and it makes them
    // agree without either shader learning the size of a texture that is nobody's
    // business but this module's.
    //
    // Forgetting it is the caller's `QUORRA_FEEDBACK.md` §11: a page zoomed past 1000%
    // and back drew one glyph's coverage under another glyph's quad — the right place,
    // the right size, the wrong letter — because the resolve read the sheet's
    // coordinates out of a texture the winding pass had stretched over a larger one.
    pass.set_viewport(
        0.0,
        0.0,
        pane.size[0].max(1) as f32,
        pane.size[1].max(1) as f32,
        0.0,
        1.0,
    );
    pass.set_pipeline(pipeline);
    pass.set_vertex_buffer(0, buffers.vertices.slice(..));
    for bind_group in &globals.samples {
        pass.set_bind_group(0, bind_group, &[]);
        // This pane's tiles' triangles, and only those. Under ADR 0027 every band drew
        // every vertex in the frame and the shader mapped the outsiders out of clip
        // space, which was affordable while a band was a shelf of the sheet; a pane can
        // be a single large tile, so the frame would have paid its whole vertex buffer
        // once per tile. The runs cost nothing to keep — the encoder appends a tile's
        // vertices contiguously — and in sheet order they coalesce back to one draw.
        for run in &pane.vertex_runs {
            pass.draw(run.clone(), 0..1);
        }
    }
}

/// One round's resolve: each tile's quad turns four samples into a quarter of its
/// coverage, added to whatever earlier rounds contributed.
#[allow(clippy::too_many_arguments)] // the pass's inputs, named once at the one call
fn resolve(
    encoder: &mut wgpu::CommandEncoder,
    coverage_view: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    buffers: &Buffers,
    globals: &PaneGlobals,
    pane: &Pane,
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
                // Cleared once, and only when this lane owns the sheet: the CPU lane's
                // bytes are already in it otherwise, and a clear would take them out.
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
    pass.set_bind_group(0, &globals.resolve, &[]);
    pass.set_bind_group(1, winding_source, &[]);
    pass.set_vertex_buffer(0, buffers.tiles.slice(..));
    // This pane's tiles, and only those: the winding target holds this pane's rectangle,
    // so another pane's quad would read texels that belong to somebody else.
    let first_tile = pane.first_tile;
    pass.draw(0..4, first_tile..first_tile.saturating_add(pane.tile_count));
}

/// One group of four samples, as the offsets they place their geometry by.
///
/// The bind groups are built per pane rather than held here: the uniform carries the
/// pane, so one per sample would be one per pane per sample, and building them where
/// they are used keeps the pane from having to be threaded into a field.
struct Group {
    offsets: Vec<[f32; 2]>,
}

impl Group {
    /// This group's uniforms for one pane: one per sample for the winding pass, and one
    /// for the resolve, which reads the sheet size and the pane and nothing else.
    fn for_pane(
        &self,
        gpu: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &PipelineStore,
        sheet: &Sheet,
        pane: &Pane,
    ) -> PaneGlobals {
        let layout = pipelines.winding_layout();
        let samples = self
            .offsets
            .iter()
            .enumerate()
            .map(|(channel, offset)| {
                globals_bind_group(gpu, queue, &layout, sheet, *offset, channel, pane)
            })
            .collect();
        let resolve = globals_bind_group(gpu, queue, &layout, sheet, [0.0, 0.0], 0, pane);
        PaneGlobals { samples, resolve }
    }
}

/// One group's uniforms, bound to one pane.
struct PaneGlobals {
    samples: Vec<wgpu::BindGroup>,
    resolve: wgpu::BindGroup,
}

/// Everything the two passes read, built once per frame.
struct Buffers {
    vertices: wgpu::Buffer,
    tiles: wgpu::Buffer,
    groups: Vec<Group>,
}

impl Buffers {
    #[allow(clippy::cast_precision_loss)] // sheet extents are far below f32's exact range
    #[allow(clippy::arithmetic_side_effects)] // a Vec length times its element count
    fn new(
        gpu: &wgpu::Device,
        queue: &wgpu::Queue,
        sheet: &Sheet,
        plan: &Plan,
        samples: u32,
    ) -> Self {
        let vertices = create_buffer(
            gpu,
            queue,
            "quorra winding vertices",
            &to_bytes(&sheet.vertices),
            wgpu::BufferUsages::VERTEX,
        );
        // Tiles go up in the plan's order, so a pane's instances are one contiguous
        // range and its resolve is one draw. The vertices stay where the encoder put
        // them — permuting the largest buffer in the frame would cost more than the
        // per-tile draw ranges the plan carries instead.
        let mut tile_data: Vec<f32> = Vec::with_capacity(sheet.tiles.len() * 6);
        for index in &plan.order {
            let tile = &sheet.tiles[*index as usize];
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

        let groups = sample_offsets(samples)
            .chunks(SAMPLES_PER_PASS as usize)
            .map(|chunk| Group {
                offsets: chunk.to_vec(),
            })
            .collect();

        Self {
            vertices,
            tiles,
            groups,
        }
    }
}

/// The 48-byte uniform one winding draw reads.
#[allow(clippy::cast_precision_loss)] // sheet extents are far below f32's exact range
fn globals_bind_group(
    gpu: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sheet: &Sheet,
    offset: [f32; 2],
    channel: usize,
    pane: &Pane,
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
        pane.origin[0] as f32,
        pane.origin[1] as f32,
        pane.size[0].max(1) as f32,
        pane.size[1].max(1) as f32,
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
    use super::{Sheet, TILE_STRIDE, render_into, sample_offsets};
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
        let outline = QuadOutline::from_segments(segments);
        let mut vertices = Vec::new();
        outline.append_triangles(
            |p| [p.x, p.y],
            [0.0, 0.0, SIDE as f32, SIDE as f32],
            &mut vertices,
        );
        let mut sheet = Sheet {
            width: SIDE,
            height: SIDE,
            ..Sheet::default()
        };
        sheet.push_tile([0.0, 0.0, SIDE as f32, SIDE as f32], even_odd, &vertices);
        let texture = gpu.create_texture(&wgpu::TextureDescriptor {
            label: Some("winding test sheet"),
            size: wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut reuse = super::WindingTexture::default();
        render_into(
            gpu,
            queue,
            device.pipeline_store(),
            &mut reuse,
            &view,
            &sheet,
            samples,
            true,
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

    /// A sheet with no tiles costs nothing, however large the extent it carries.
    ///
    /// The extent is the scratch sheet's and arrives on every frame that packs a tile,
    /// so this is the ordinary shape of a CPU-lane frame — not an edge case. Charging
    /// it `width × height × 8` for an `rgba16float` texture nothing asks for is what
    /// refused five real pages; `tests/coverage_lanes.rs` holds the same invariant end
    /// to end.
    #[test]
    fn a_sheet_with_no_tiles_costs_nothing_however_large_its_extent() {
        let empty = Sheet {
            width: 16384,
            height: 8760,
            ..Sheet::default()
        };
        assert!(empty.is_empty());
        assert_eq!(empty.device_bytes(), 0);

        // And the target is priced the moment a tile makes the texture real. This sheet
        // is 64 × 4, well inside the pane budget, so the target is the whole of it —
        // which is what makes this the same arithmetic it was before panes.
        let mut one_tile = Sheet {
            width: 64,
            height: 4,
            ..Sheet::default()
        };
        let mut vertices = Vec::new();
        QuadOutline::from_segments(&rect_path(0.0, 0.0, 4.0, 4.0)).append_triangles(
            |p| [p.x, p.y],
            [0.0, 0.0, 4.0, 4.0],
            &mut vertices,
        );
        one_tile.push_tile([0.0, 0.0, 4.0, 4.0], false, &vertices);
        let vertex_count = vertices.len() as u64;
        assert_eq!(
            one_tile.device_bytes(),
            64 * 4 * 8 + vertex_count * crate::outline::WindingVertex::STRIDE + TILE_STRIDE
        );
    }
}

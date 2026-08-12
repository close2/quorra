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

/// One horizontal slice of the sheet: the rows the winding target holds at a time, and
/// the run of tiles whose coverage they resolve (ADR 0027).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Band {
    /// First sheet row this band covers.
    pub origin: u32,
    /// How many rows — at least the tallest tile in it, since a tile may not be split.
    pub height: u32,
    /// The band's tiles, as a range into the *sorted* tile order `Buffers` uploads.
    pub first_tile: u32,
    pub tile_count: u32,
}

/// How many bytes of winding target a frame may hold at once.
///
/// The target is scratch — accumulated, resolved into the R8 sheet, and dead — so its
/// size is a choice, and before ADR 0027 the choice was "the whole sheet", at eight
/// bytes a texel. That refused a page of sixty large shapes at 359 MB. Sixteen mebibytes
/// is two thousand rows of a page-wide sheet and one row of a 16 384-wide one; a band is
/// never smaller than its tallest tile, so this is a target rather than a bound.
const BAND_BYTES: u64 = 16 * 1024 * 1024;

impl Sheet {
    /// The bands this sheet's tiles fall into, in sheet order.
    ///
    /// Greedy over tiles sorted by their top row: a band grows until adding the next
    /// tile would take it past [`BAND_BYTES`], and never splits one — a tile taller than
    /// the budget is a band of its own, which is why the winding target is sized from
    /// the tallest band rather than from the constant.
    ///
    /// The returned ranges index the sorted order, which is what [`Buffers`] uploads:
    /// the *vertices* are not sorted and do not need to be, since every band draws all
    /// of them and the shader maps the ones outside it out of clip space.
    pub(crate) fn bands(&self) -> Vec<Band> {
        let mut order: Vec<usize> = (0..self.tiles.len()).collect();
        order.sort_by(|a, b| {
            self.tiles[*a].rect[1]
                .total_cmp(&self.tiles[*b].rect[1])
                .then_with(|| self.tiles[*a].rect[0].total_cmp(&self.tiles[*b].rect[0]))
        });
        // Sheet coordinates are integers stored as floats — the packer places tiles on
        // whole texels — so this narrowing is exact for every value that reaches it.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let row = |value: f32| value.max(0.0) as u32;
        let budget_rows = BAND_BYTES
            .checked_div(u64::from(self.width.max(1)).saturating_mul(8))
            .unwrap_or(1)
            .max(1);
        let mut bands: Vec<Band> = Vec::new();
        for (position, index) in order.iter().enumerate() {
            let tile = &self.tiles[*index];
            let (top, bottom) = (row(tile.rect[1]), row(tile.rect[3].ceil()));
            match bands.last_mut() {
                // Fits the open band, or is taller than any budget and so extends it:
                // either way the band grows to hold it whole.
                Some(open)
                    if u64::from(bottom.saturating_sub(open.origin)) <= budget_rows
                        || open.tile_count == 0 =>
                {
                    open.height = bottom.saturating_sub(open.origin).max(open.height);
                    open.tile_count = open.tile_count.saturating_add(1);
                }
                _ => bands.push(Band {
                    origin: top,
                    height: bottom.saturating_sub(top).max(1),
                    // A frame with more tiles than a `u32` counts was refused by the
                    // budget long before it packed them.
                    first_tile: u32::try_from(position).unwrap_or(u32::MAX),
                    tile_count: 1,
                }),
            }
        }
        bands
    }

    /// The tallest band, which is what the winding target must hold.
    pub(crate) fn band_rows(&self) -> u32 {
        self.bands()
            .iter()
            .map(|band| band.height)
            .max()
            .unwrap_or(self.height)
            .min(self.height.max(1))
    }

    /// Adds a tile and the triangles that fill it.
    pub(crate) fn push_tile(&mut self, tile: Tile, vertices: &[crate::outline::WindingVertex]) {
        // A tile with no triangles is not a tile: it would resolve to transparent,
        // which the sheet already is there.
        if vertices.is_empty() {
            return;
        }
        for vertex in vertices {
            self.vertices.extend_from_slice(&vertex.floats());
        }
        self.tiles.push(tile);
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
        // The winding target holds one *band*, not the sheet (ADR 0027).
        let texels = u64::from(self.width).saturating_mul(u64::from(self.band_rows()));
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
    // One band at a time (ADR 0027): the target is scratch, so it is sized by what a
    // pass holds rather than by what the page came to.
    let extent = wgpu::Extent3d {
        width: sheet.width.max(1),
        height: sheet.band_rows().max(1),
        depth_or_array_layers: 1,
    };
    // **Kept between frames**, which ADR 0012 declined to do for the compositor's
    // textures "until a measurement says otherwise". This is that measurement: at 20x
    // the sheet is 2.5 million texels, and allocating and zero-initialising eight
    // bytes of each, every frame, cost 10.7 ms of a 15 ms frame — more than the
    // rasterising the lane exists to avoid. One texture, grown when a frame needs a
    // larger one and never shrunk while the device lives.
    let winding_view = reuse.view_for(gpu, extent).clone();

    let buffers = Buffers::new(gpu, queue, sheet, samples);
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
    for (index, band) in buffers.bands.iter().enumerate() {
        for (round, group) in buffers.groups.iter().enumerate() {
            let globals = group.for_band(gpu, queue, pipelines, sheet, *band);
            accumulate(
                &mut encoder,
                &winding_view,
                &winding_pipeline,
                &buffers,
                &globals,
                sheet.width.max(1),
                band.height.max(1),
            );
            resolve(
                &mut encoder,
                coverage_view,
                &resolve_pipeline,
                &buffers,
                &globals,
                *band,
                &winding_source,
                // The coverage sheet is cleared once, by the first pass that touches
                // it, and only when this lane owns it: every later band and round adds
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
/// **The frame's sheet is the top-left of it**, whatever the rest of it is. Growing and
/// never shrinking is what makes that a thing to state rather than a tautology: a
/// smaller frame after a larger one gets a texture with room to spare, and `fs_resolve`
/// reads sheet coordinates as texels of this texture. [`accumulate`]'s viewport is what
/// puts them there; `tests/frame_independence.rs` is what keeps them there.
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

/// One round's winding pass: clear, then one draw per sample of the group.
///
/// `sheet` is the extent this frame's sheet occupies, which is **not** the attachment's
/// size: [`WindingTexture`] is kept between frames and is at least as large as any sheet
/// it has held. See the viewport below for what that costs if it is forgotten.
#[allow(clippy::cast_precision_loss)] // an extent bounded by the adapter's texture limit
fn accumulate(
    encoder: &mut wgpu::CommandEncoder,
    winding_view: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    buffers: &Buffers,
    globals: &BandGlobals,
    width: u32,
    height: u32,
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
    // **The sheet is the top-left of this texture, not the whole of it.** `vs_winding`
    // divides by the *sheet* size to reach clip space, and clip space spans whatever is
    // attached — so with a texture kept from a taller frame, and no viewport, every
    // sheet pixel would be written `held / sheet` times further down than the resolve
    // pass reads it. The viewport is what makes the two agree, and it makes them agree
    // without either shader learning the size of a texture that is nobody's business
    // but this module's.
    //
    // Forgetting it is the caller's `QUORRA_FEEDBACK.md` §11: a page zoomed past 1000%
    // and back drew one glyph's coverage under another glyph's quad — the right place,
    // the right size, the wrong letter — because the resolve read the sheet's
    // coordinates out of a texture the winding pass had stretched over a larger one.
    //
    // With bands (ADR 0027) the viewport is the *band's* extent at the target's origin,
    // and `vs_winding` subtracts the band's first row before mapping — the same
    // agreement, one subtraction deeper.
    pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
    pass.set_pipeline(pipeline);
    pass.set_vertex_buffer(0, buffers.vertices.slice(..));
    for bind_group in &globals.samples {
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..buffers.vertex_count, 0..1);
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
    globals: &BandGlobals,
    band: Band,
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
    // This band's tiles, and only those: the winding target holds this band's rows, so
    // another band's quad would read a row that belongs to somebody else.
    let first_tile = band.first_tile;
    pass.draw(0..4, first_tile..first_tile.saturating_add(band.tile_count));
}

/// One group of four samples, as the offsets they place their geometry by.
///
/// The bind groups are built per band rather than held here (ADR 0027): the uniform
/// carries the band, so one per sample would be one per band per sample, and building
/// them where they are used keeps the band from having to be threaded into a field.
struct Group {
    offsets: Vec<[f32; 2]>,
}

impl Group {
    /// This group's uniforms for one band: one per sample for the winding pass, and one
    /// for the resolve, which reads the sheet size and the band and nothing else.
    fn for_band(
        &self,
        gpu: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &PipelineStore,
        sheet: &Sheet,
        band: Band,
    ) -> BandGlobals {
        let layout = pipelines.winding_layout();
        let samples = self
            .offsets
            .iter()
            .enumerate()
            .map(|(channel, offset)| {
                globals_bind_group(gpu, queue, &layout, sheet, *offset, channel, band)
            })
            .collect();
        let resolve = globals_bind_group(gpu, queue, &layout, sheet, [0.0, 0.0], 0, band);
        BandGlobals { samples, resolve }
    }
}

/// One group's uniforms, bound to one band.
struct BandGlobals {
    samples: Vec<wgpu::BindGroup>,
    resolve: wgpu::BindGroup,
}

/// Everything the two passes read, built once per frame.
struct Buffers {
    vertices: wgpu::Buffer,
    vertex_count: u32,
    tiles: wgpu::Buffer,
    groups: Vec<Group>,
    /// The sheet's rows, sliced into what the winding target holds at once, with each
    /// band's tiles a contiguous run of the instance buffer below (ADR 0027).
    bands: Vec<Band>,
}

impl Buffers {
    #[allow(clippy::cast_precision_loss)] // sheet extents are far below f32's exact range
    #[allow(clippy::cast_possible_truncation)] // a frame with 2^32 vertices was refused
    // by the budget long before it reached this cast
    #[allow(clippy::arithmetic_side_effects)] // a Vec length times its element count
    fn new(gpu: &wgpu::Device, queue: &wgpu::Queue, sheet: &Sheet, samples: u32) -> Self {
        let vertices = create_buffer(
            gpu,
            queue,
            "quorra winding vertices",
            &to_bytes(&sheet.vertices),
            wgpu::BufferUsages::VERTEX,
        );
        // Tiles go up in band order, so a band's instances are one contiguous range and
        // its resolve is one draw. The vertices are *not* reordered: every band draws
        // all of them and the shader maps the ones outside it out of clip space, which
        // costs vertex work and saves a permutation of the largest buffer in the frame.
        let bands = sheet.bands();
        let mut order: Vec<usize> = (0..sheet.tiles.len()).collect();
        order.sort_by(|a, b| {
            sheet.tiles[*a].rect[1]
                .total_cmp(&sheet.tiles[*b].rect[1])
                .then_with(|| sheet.tiles[*a].rect[0].total_cmp(&sheet.tiles[*b].rect[0]))
        });
        let mut tile_data: Vec<f32> = Vec::with_capacity(sheet.tiles.len() * 6);
        for index in &order {
            let tile = &sheet.tiles[*index];
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
            vertex_count: (sheet.vertices.len() / 8) as u32,
            vertices,
            tiles,
            groups,
            bands,
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
    band: Band,
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
        band.origin as f32,
        band.height.max(1) as f32,
        0.0,
        0.0,
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
    use super::{Sheet, Tile, render_into, sample_offsets};
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

        // And the extent is priced in full the moment a tile makes the texture real:
        // one triangle of eight floats a vertex, plus the texture, plus the tile.
        let one_tile = Sheet {
            vertices: vec![0.0; 3 * 8],
            tiles: vec![Tile {
                rect: [0.0, 0.0, 4.0, 4.0],
                even_odd: false,
            }],
            width: 64,
            height: 4,
        };
        assert_eq!(
            one_tile.device_bytes(),
            64 * 4 * 8 + 3 * crate::outline::WindingVertex::STRIDE + super::TILE_STRIDE
        );
    }
}

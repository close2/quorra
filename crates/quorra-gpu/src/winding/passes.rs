//! The winding target, and the two passes that write and read it.
//!
//! They are one module because they share one invariant that neither of them states on
//! its own: **the pane being drawn is the top-left of the texture**, whatever the rest
//! of it is. The texture is kept between frames and only ever grows, so it is usually
//! larger than the pane; [`accumulate`]'s viewport is what puts the pane's pixels where
//! [`resolve`] reads them, and the caller's `QUORRA_FEEDBACK.md` §11 is what happens
//! when the two are read apart.

use crate::pane::Pane;

use super::{Buffers, PaneGlobals, WINDING_FORMAT};

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
    pub(super) fn view_for(
        &mut self,
        gpu: &wgpu::Device,
        extent: wgpu::Extent3d,
    ) -> &wgpu::TextureView {
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
pub(super) fn accumulate(
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
pub(super) fn resolve(
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

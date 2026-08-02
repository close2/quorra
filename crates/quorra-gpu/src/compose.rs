//! The frame executor: layers, composites, mask reductions, and the passes that
//! carry them, recorded into one submission.
//!
//! ISO 32000-2 clause 11, run: each [`LayerPlan`] renders bottom-up into its own
//! premultiplied texture; a [`ChildOp`] closes the current pass, renders the child,
//! and composites it onto the accumulated parent through `composite.wgsl` — a
//! ping-pong between the parent's two textures, because a pass cannot read its own
//! attachment. Soft masks realise first, in id order (they may only reference
//! earlier masks), each reduced to R8 by `reduce.wgsl`. A frame whose root has no
//! children and no masks draws straight into the target — the flat fast path M1
//! measured stays exactly as cheap as it was.
//!
//! **Count then allocate** (§5): every layer texture and mask texture is priced
//! against the frame budget before anything is created; the refusal names both
//! numbers. Knockout batches run their erase/add pair strictly per element
//! (ADR 0010): interleaving is what makes overlapping knockout elements compose per
//! §11.4.6 rather than approximately.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::device::{Device, PassQuery};
use crate::encode::{Batch, BatchKind, ChildOp, DrawStyle, Encoded, ImageOp, Op, ShadedOp};
use crate::error::RenderError;
use crate::pipeline::Kind;

/// One drawable item of a pass: an instanced lane batch, or a single-quad op
/// (image, shading, mesh — ADR 0011's rare cases).
pub(crate) enum RunOp {
    Batch(Batch),
    Image(ImageOp),
    Shaded(ShadedOp),
}

/// A prepared item of a pass: everything a draw needs that cannot be made while
/// the pass borrows the recorder.
enum Ready {
    Batch(Batch),
    /// Over is one pipeline; knockout is the erase/add pair, in order.
    Single {
        kinds: [Option<Kind>; 2],
        bind: wgpu::BindGroup,
    },
}

/// The drawable prefix of a plan's ops, unboxed for a pass. The caller guarantees
/// the slice holds no `Op::Child`.
pub(crate) fn run_ops(ops: &[Op]) -> Vec<RunOp> {
    ops.iter()
        .map(|op| match op {
            Op::Draw(batch) => RunOp::Batch(*batch),
            Op::Image(image) => RunOp::Image(**image),
            Op::Shaded(shaded) => RunOp::Shaded(**shaded),
            // The callers collect runs of drawable ops only.
            Op::Child(_) => unreachable!("a draw run contains no child composites"),
        })
        .collect()
}

/// Everything the executor holds for one frame.
pub(crate) struct Executor<'a> {
    pub device: &'a Device,
    pub encoded: &'a Encoded,
    pub width: u32,
    pub height: u32,
    /// Per-plan texture pairs: index 0 is the root, 1.. are `Encoded::layers`.
    pub pairs: Vec<[wgpu::Texture; 2]>,
    /// Realised mask views by mask index.
    pub mask_views: Vec<Option<wgpu::TextureView>>,
    /// Lane instance buffers.
    pub rect_buffer: Option<wgpu::Buffer>,
    pub quad_buffer: Option<wgpu::Buffer>,
    pub globals_bind: wgpu::BindGroup,
    /// Lane bind groups per mask index (`None` key = no mask), built lazily.
    pub lane_binds: HashMap<Option<u32>, wgpu::BindGroup>,
    pub scratch_view: Option<wgpu::TextureView>,
    pub dummy_view: wgpu::TextureView,
    pub atlas_view: Option<wgpu::TextureView>,
    pub first_pass_stamped: bool,
    pub query: Option<&'a PassQuery>,
    pub phases: Vec<(&'static str, Duration)>,
    /// When a frame patches a damage list (ADR 0012), every internal pass is
    /// scissored to the damage bounding box — every pass in this pipeline is
    /// pixel-local (a fragment reads attachments only at its own coordinate), so
    /// pixels outside the scissor are never needed and never paid for.
    pub scissor: Option<[u32; 4]>,
}

/// How many internal textures this frame needs, for the budget check before any of
/// them exist: two per plan (ping-pong) when layers are needed at all, plus one
/// RGBA pair set and one R8 per used mask. `force_layers` prices the root pair a
/// damage-patched flat frame renders through (ADR 0012).
pub(crate) fn internal_texture_bytes(
    encoded: &Encoded,
    width: u32,
    height: u32,
    force_layers: bool,
) -> u64 {
    let per_layer = u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(4);
    let masks_used = encoded.mask_plans.iter().flatten().count() as u64;
    let needs_layers = !encoded.layers.is_empty() || masks_used > 0 || force_layers;
    if !needs_layers {
        return 0;
    }
    let plan_count = (encoded.layers.len() as u64).saturating_add(1);
    plan_count
        .saturating_mul(2)
        .saturating_mul(per_layer)
        .saturating_add(masks_used.saturating_mul(per_layer / 4))
}

impl Executor<'_> {
    /// Whether the frame can skip layers entirely and draw the root straight into
    /// the target.
    pub(crate) fn is_flat(encoded: &Encoded) -> bool {
        encoded.layers.is_empty() && encoded.mask_plans.iter().all(Option::is_none)
    }

    /// Realise every used soft mask, in id order (§11.5), into R8 views.
    pub(crate) fn realise_masks(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
    ) -> Result<(), RenderError> {
        for index in 0..self.encoded.mask_plans.len() {
            let Some(plan) = &self.encoded.mask_plans[index] else {
                continue;
            };
            let group_view = self.render_plan(recorder, plan.root.saturating_add(1))?;
            let mask_texture = self.device.create_internal_texture(
                "quorra soft mask",
                self.width,
                self.height,
                wgpu::TextureFormat::R8Unorm,
            );
            let mask_view = mask_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind = self.device.reduce_bind(plan, &group_view);
            let (pipeline, compiled) = self
                .device
                .pipelines()
                .get(Kind::Reduce, wgpu::TextureFormat::R8Unorm);
            if let Some(duration) = compiled {
                self.phases.push(("pipeline compile (first use)", duration));
            }
            let mut pass = recorder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("quorra reduce"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &mask_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: self.pass_stamp(),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.apply_scissor(&mut pass);
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            self.mask_views[index] = Some(mask_view);
        }
        Ok(())
    }

    /// Render one plan into its texture pair; returns the view holding the result.
    ///
    /// `pair_index` 0 is the root, `layers[i]` is `i + 1`. Recursion depth is the
    /// scene's group depth, bounded at 16 by the builder.
    pub(crate) fn render_plan(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
        pair_index: usize,
    ) -> Result<wgpu::TextureView, RenderError> {
        let plan = if pair_index == 0 {
            &self.encoded.root
        } else {
            &self.encoded.layers[pair_index.saturating_sub(1)]
        };
        let mut current = 0_usize;
        let mut cleared = false;
        let mut op_index = 0;
        while op_index < plan.ops.len() {
            match &plan.ops[op_index] {
                Op::Draw(_) | Op::Image(_) | Op::Shaded(_) => {
                    // A run of consecutive drawable ops becomes one pass.
                    let run_start = op_index;
                    while op_index < plan.ops.len() && !matches!(plan.ops[op_index], Op::Child(_)) {
                        op_index = op_index.saturating_add(1);
                    }
                    let view = self.pairs[pair_index][current]
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    let run = run_ops(&plan.ops[run_start..op_index]);
                    self.draw_pass(
                        recorder,
                        &view,
                        wgpu::TextureFormat::Rgba8Unorm,
                        !cleared,
                        &run,
                    )?;
                    cleared = true;
                }
                Op::Child(child) => {
                    let child_op = *child;
                    op_index = op_index.saturating_add(1);
                    if !cleared {
                        // The composite reads the backdrop, so it must exist even if
                        // nothing was drawn yet: clear it with an empty pass.
                        let view = self.pairs[pair_index][current]
                            .create_view(&wgpu::TextureViewDescriptor::default());
                        self.draw_pass(
                            recorder,
                            &view,
                            wgpu::TextureFormat::Rgba8Unorm,
                            true,
                            &[],
                        )?;
                        cleared = true;
                    }
                    let child_view =
                        self.render_plan(recorder, child_op.layer.saturating_add(1))?;
                    let backdrop_view = self.pairs[pair_index][current]
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    let flip = 1_usize.saturating_sub(current);
                    let out_view = self.pairs[pair_index][flip]
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    self.composite_pass(
                        recorder,
                        &out_view,
                        &backdrop_view,
                        &child_view,
                        &child_op,
                    );
                    current = flip;
                }
            }
        }
        if !cleared {
            let view = self.pairs[pair_index][current]
                .create_view(&wgpu::TextureViewDescriptor::default());
            self.draw_pass(recorder, &view, wgpu::TextureFormat::Rgba8Unorm, true, &[])?;
        }
        Ok(self.pairs[pair_index][current].create_view(&wgpu::TextureViewDescriptor::default()))
    }

    /// One render pass of lane batches and single-quad ops onto `view`. Public to
    /// the device so the flat fast path draws the root directly onto the frame's
    /// target.
    pub(crate) fn draw_pass(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        clear: bool,
        ops: &[RunOp],
    ) -> Result<(), RenderError> {
        let (ready, needed) = self.prepare_run(ops)?;
        let mut pipelines = HashMap::new();
        for kind in needed {
            let (pipeline, compiled) = self.device.pipelines().get(kind, format);
            if let Some(duration) = compiled {
                self.phases.push(("pipeline compile (first use)", duration));
            }
            pipelines.insert(kind, pipeline);
        }
        let stamp = self.pass_stamp();
        let mut pass = recorder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("quorra content"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Render onto transparency, always (§3; §11.4.7).
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: stamp,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.apply_scissor(&mut pass);
        for item in &ready {
            let batch = match item {
                Ready::Batch(batch) => batch,
                Ready::Single { kinds, bind } => {
                    for kind in kinds.iter().flatten() {
                        pass.set_pipeline(&pipelines[kind]);
                        pass.set_bind_group(0, bind, &[]);
                        pass.draw(0..4, 0..1);
                    }
                    continue;
                }
            };
            let buffer = match batch.kind {
                BatchKind::Rect => self.rect_buffer.as_ref(),
                BatchKind::Quad => self.quad_buffer.as_ref(),
            };
            let Some(buffer) = buffer else { continue };
            let bind = &self.lane_binds[&batch.mask];
            match batch.style {
                DrawStyle::Over => {
                    let kind = match batch.kind {
                        BatchKind::Rect => Kind::RectOver,
                        BatchKind::Quad => Kind::CoverOver,
                    };
                    pass.set_pipeline(&pipelines[&kind]);
                    pass.set_bind_group(0, &self.globals_bind, &[]);
                    pass.set_bind_group(1, bind, &[]);
                    pass.set_vertex_buffer(0, buffer.slice(..));
                    pass.draw(0..4, batch.first..batch.first.saturating_add(batch.count));
                }
                DrawStyle::Knockout => {
                    // §11.4.6 per element: erase by shape, then deposit — strictly
                    // interleaved, or overlapping elements compose wrongly
                    // (ADR 0010 carries the algebra).
                    let (erase_kind, add_kind) = match batch.kind {
                        BatchKind::Rect => (Kind::RectErase, Kind::RectAdd),
                        BatchKind::Quad => (Kind::CoverErase, Kind::CoverAdd),
                    };
                    for i in batch.first..batch.first.saturating_add(batch.count) {
                        pass.set_pipeline(&pipelines[&erase_kind]);
                        pass.set_bind_group(0, &self.globals_bind, &[]);
                        pass.set_bind_group(1, bind, &[]);
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        pass.draw(0..4, i..i.saturating_add(1));
                        pass.set_pipeline(&pipelines[&add_kind]);
                        pass.set_bind_group(0, &self.globals_bind, &[]);
                        pass.set_bind_group(1, bind, &[]);
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        pass.draw(0..4, i..i.saturating_add(1));
                    }
                }
            }
        }
        Ok(())
    }

    /// Phase one of a draw pass: build every bind group and collect every pipeline
    /// kind the run needs — none of which can happen while the pass borrows the
    /// recorder.
    fn prepare_run(&mut self, ops: &[RunOp]) -> Result<(Vec<Ready>, Vec<Kind>), RenderError> {
        let mut needed: Vec<Kind> = Vec::new();
        let want = |needed: &mut Vec<Kind>, kind: Kind| {
            if !needed.contains(&kind) {
                needed.push(kind);
            }
        };
        let mut ready: Vec<Ready> = Vec::with_capacity(ops.len());
        for op in ops {
            match op {
                RunOp::Batch(batch) => {
                    for kind in batch_kinds(batch) {
                        want(&mut needed, kind);
                    }
                    self.ensure_lane_bind(batch.mask);
                    ready.push(Ready::Batch(*batch));
                }
                RunOp::Image(image) => {
                    let kinds = style_kinds(
                        image.style,
                        Kind::ImageOver,
                        Kind::ImageErase,
                        Kind::ImageAdd,
                    );
                    for kind in kinds.iter().flatten() {
                        want(&mut needed, *kind);
                    }
                    let mask_view = image
                        .mask
                        .and_then(|m| self.mask_views[m as usize].as_ref())
                        .unwrap_or(&self.dummy_view);
                    let scratch = self.scratch_view.as_ref().unwrap_or(&self.dummy_view);
                    let bind = self.device.image_bind(
                        image,
                        self.width,
                        self.height,
                        mask_view,
                        scratch,
                    )?;
                    ready.push(Ready::Single { kinds, bind });
                }
                RunOp::Shaded(shaded) => {
                    let kinds = style_kinds(
                        shaded.style,
                        Kind::ShadedOver,
                        Kind::ShadedErase,
                        Kind::ShadedAdd,
                    );
                    for kind in kinds.iter().flatten() {
                        want(&mut needed, *kind);
                    }
                    let mask_view = shaded
                        .mask
                        .and_then(|m| self.mask_views[m as usize].as_ref())
                        .unwrap_or(&self.dummy_view);
                    let scratch = self.scratch_view.as_ref().unwrap_or(&self.dummy_view);
                    let bind = self.device.shaded_bind(
                        shaded,
                        self.width,
                        self.height,
                        scratch,
                        mask_view,
                    )?;
                    ready.push(Ready::Single { kinds, bind });
                }
            }
        }
        Ok((ready, needed))
    }

    /// One composite pass: `out = child over/blended-onto backdrop` per §11.3.6.
    fn composite_pass(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
        out: &wgpu::TextureView,
        backdrop: &wgpu::TextureView,
        child: &wgpu::TextureView,
        op: &ChildOp,
    ) {
        let mask_view = op
            .mask
            .and_then(|m| self.mask_views[m as usize].as_ref())
            .unwrap_or(&self.dummy_view);
        let scratch = self.scratch_view.as_ref().unwrap_or(&self.dummy_view);
        let bind = self
            .device
            .composite_bind(op, backdrop, child, mask_view, scratch);
        let (pipeline, compiled) = self
            .device
            .pipelines()
            .get(Kind::Composite, wgpu::TextureFormat::Rgba8Unorm);
        if let Some(duration) = compiled {
            self.phases.push(("pipeline compile (first use)", duration));
        }
        let stamp = self.pass_stamp();
        let mut pass = recorder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("quorra composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: out,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: stamp,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.apply_scissor(&mut pass);
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Blit the finished root onto the frame's target.
    pub(crate) fn blit_to_target(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
        src: &wgpu::TextureView,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) {
        let bind = self.device.blit_bind(src);
        let (pipeline, compiled) = self.device.pipelines().get(Kind::Blit, format);
        if let Some(duration) = compiled {
            self.phases.push(("pipeline compile (first use)", duration));
        }
        let stamp = self.pass_stamp();
        let mut pass = recorder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("quorra blit"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: stamp,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Patch the finished root onto the target: one scissored REPLACE blit per
    /// damage rectangle, over the target's retained contents (`LoadOp::Load`).
    /// Nothing outside the rectangles is written — that is the whole contract.
    pub(crate) fn patch_to_target(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
        src: &wgpu::TextureView,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        rects: &[[u32; 4]],
    ) {
        let bind = self.device.blit_bind(src);
        let (pipeline, compiled) = self.device.pipelines().get(Kind::Blit, format);
        if let Some(duration) = compiled {
            self.phases.push(("pipeline compile (first use)", duration));
        }
        let stamp = self.pass_stamp();
        let mut pass = recorder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("quorra damage patch"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: stamp,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        for &[x, y, w, h] in rects {
            pass.set_scissor_rect(x, y, w, h);
            pass.draw(0..3, 0..1);
        }
    }

    /// Scissor an internal pass to the damage bounding box, when this frame
    /// patches.
    fn apply_scissor(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some([x, y, w, h]) = self.scissor {
            pass.set_scissor_rect(x, y, w, h);
        }
    }

    /// The end-of-frame timestamp: an empty pass whose only job is the second
    /// timestamp, so `execute` spans first pass to last whatever the pass count.
    pub(crate) fn end_stamp(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    ) {
        let Some(query) = self.query else { return };
        let mut _pass = recorder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("quorra end stamp"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                query_set: &query.set,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: Some(1),
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }

    /// The first pass stamps the frame's beginning; later passes stamp nothing (the
    /// end stamp is its own pass).
    fn pass_stamp(&mut self) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        if self.first_pass_stamped {
            return None;
        }
        self.first_pass_stamped = true;
        self.query.map(|q| wgpu::RenderPassTimestampWrites {
            query_set: &q.set,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: None,
        })
    }

    /// Build (once per mask) the lane bind group carrying atlas, scratch and the
    /// mask's view.
    fn ensure_lane_bind(&mut self, mask: Option<u32>) {
        if self.lane_binds.contains_key(&mask) {
            return;
        }
        let mask_view = mask
            .and_then(|m| self.mask_views[m as usize].as_ref())
            .unwrap_or(&self.dummy_view);
        let atlas = self.atlas_view.as_ref().unwrap_or(&self.dummy_view);
        let scratch = self.scratch_view.as_ref().unwrap_or(&self.dummy_view);
        let bind = self.device.lane_bind(atlas, scratch, mask_view);
        self.lane_binds.insert(mask, bind);
    }
}

fn batch_kinds(batch: &Batch) -> Vec<Kind> {
    match (batch.kind, batch.style) {
        (BatchKind::Rect, DrawStyle::Over) => vec![Kind::RectOver],
        (BatchKind::Rect, DrawStyle::Knockout) => vec![Kind::RectErase, Kind::RectAdd],
        (BatchKind::Quad, DrawStyle::Over) => vec![Kind::CoverOver],
        (BatchKind::Quad, DrawStyle::Knockout) => vec![Kind::CoverErase, Kind::CoverAdd],
    }
}

/// A single-quad op's pipelines for its style: over alone, or the knockout
/// erase/add pair in ADR 0010's strict order.
fn style_kinds(style: DrawStyle, over: Kind, erase: Kind, add: Kind) -> [Option<Kind>; 2] {
    match style {
        DrawStyle::Over => [Some(over), None],
        DrawStyle::Knockout => [Some(erase), Some(add)],
    }
}

/// Wall-clock the executor's whole submission, for the fallback `execute`.
pub(crate) fn submit_and_wait(
    device: &Device,
    recorder: wgpu::CommandEncoder,
) -> Result<Duration, RenderError> {
    let started = Instant::now();
    device.queue().submit([recorder.finish()]);
    device
        .gpu()
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| RenderError::DeviceLost {
            detail: e.to_string(),
        })?;
    Ok(started.elapsed())
}

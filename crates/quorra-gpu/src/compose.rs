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
use crate::layers::{LayerPool, Pair};
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
    /// The frame's layer textures, acquired per plan and given back when the parent's
    /// composite has read them (ADR 0020).
    pub pool: LayerPool,
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

/// One plan's finished pixels, and the pair they are in — returned rather than the
/// view alone, because the caller is what gives the pair back (ADR 0020).
pub(crate) struct Rendered {
    pair: Pair,
    current: usize,
}

impl Rendered {
    /// The texture holding this plan's result.
    pub(crate) fn view(&self) -> wgpu::TextureView {
        self.pair[self.current].create_view(&wgpu::TextureViewDescriptor::default())
    }
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
            // A soft mask's group renders on its own, onto transparency (§11.5).
            let group = self.render_plan(recorder, plan.root.saturating_add(1), None)?;
            let group_view = group.view();
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
            // The reduce has read it; the mask's own R8 is what outlives this loop.
            self.pool.release(group.pair);
            self.mask_views[index] = Some(mask_view);
        }
        Ok(())
    }

    /// Render one plan into a pair borrowed from the pool; returns the pair and which
    /// of its two textures holds the result, for the caller to read and then release
    /// (ADR 0020).
    ///
    /// `plan_index` 0 is the root, `layers[i]` is `i + 1`. Recursion depth is the plan
    /// tree's depth, which the scene builder bounds.
    ///
    /// `seed` is §11.4.4's initial backdrop for a non-isolated group: the parent's
    /// accumulated view, blitted in before anything draws (ADR 0019). `None` is
    /// §11.4.5's transparency, and then the first pass clears instead.
    pub(crate) fn render_plan(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
        plan_index: usize,
        seed: Option<&wgpu::TextureView>,
    ) -> Result<Rendered, RenderError> {
        let plan = if plan_index == 0 {
            &self.encoded.root
        } else {
            &self.encoded.layers[plan_index.saturating_sub(1)]
        };
        let pair = self.pool.acquire(self.device, self.width, self.height);
        let mut current = 0_usize;
        let mut cleared = false;
        if let Some(backdrop) = seed {
            let view = pair[0].create_view(&wgpu::TextureViewDescriptor::default());
            self.seed_layer(recorder, backdrop, &view);
            cleared = true;
        }
        let mut op_index = 0;
        while op_index < plan.ops.len() {
            match &plan.ops[op_index] {
                Op::Draw(_) | Op::Image(_) | Op::Shaded(_) => {
                    // A run of consecutive drawable ops becomes one pass.
                    let run_start = op_index;
                    while op_index < plan.ops.len() && !matches!(plan.ops[op_index], Op::Child(_)) {
                        op_index = op_index.saturating_add(1);
                    }
                    let view = pair[current].create_view(&wgpu::TextureViewDescriptor::default());
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
                    let backdrop_view =
                        pair[current].create_view(&wgpu::TextureViewDescriptor::default());
                    if !cleared {
                        // The composite reads the backdrop, so it must exist even if
                        // nothing was drawn yet: clear it with an empty pass.
                        self.draw_pass(
                            recorder,
                            &backdrop_view,
                            wgpu::TextureFormat::Rgba8Unorm,
                            true,
                            &[],
                        )?;
                        cleared = true;
                    }
                    // §11.4.4: a non-isolated group's elements composite onto the
                    // group's backdrop, so its buffer begins as a copy of what is under
                    // it. The composite that follows takes that contribution back out
                    // (ADR 0019), which is why this seeding is only half a change.
                    let seed = (!child_op.isolated).then_some(&backdrop_view);
                    let child =
                        self.render_plan(recorder, child_op.layer.saturating_add(1), seed)?;
                    let flip = 1_usize.saturating_sub(current);
                    let out_view = pair[flip].create_view(&wgpu::TextureViewDescriptor::default());
                    self.composite_pass(
                        recorder,
                        &out_view,
                        &backdrop_view,
                        &child.view(),
                        &child_op,
                    );
                    // Every pass that reads the child has been recorded; a sibling may
                    // have its textures now.
                    self.pool.release(child.pair);
                    current = flip;
                }
            }
        }
        if !cleared {
            let view = pair[current].create_view(&wgpu::TextureViewDescriptor::default());
            self.draw_pass(recorder, &view, wgpu::TextureFormat::Rgba8Unorm, true, &[])?;
        }
        Ok(Rendered { pair, current })
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
            let (erase_kind, add_kind, over_kind) = match batch.kind {
                BatchKind::Rect => (Kind::RectErase, Kind::RectAdd, Kind::RectOver),
                BatchKind::Quad => (Kind::CoverErase, Kind::CoverAdd, Kind::CoverOver),
            };
            let draw =
                |pass: &mut wgpu::RenderPass<'_>, kind: &Kind, range: std::ops::Range<u32>| {
                    pass.set_pipeline(&pipelines[kind]);
                    pass.set_bind_group(0, &self.globals_bind, &[]);
                    pass.set_bind_group(1, bind, &[]);
                    pass.set_vertex_buffer(0, buffer.slice(..));
                    pass.draw(0..4, range);
                };
            let whole = batch.first..batch.first.saturating_add(batch.count);
            match batch.style {
                DrawStyle::Over => draw(&mut pass, &over_kind, whole),
                // One stage of §11.4.6, asked for by name (ADR 0025): the batch is
                // instanced like any other, because a single pass over independent
                // marks needs no interleaving.
                DrawStyle::DestOut => draw(&mut pass, &erase_kind, whole),
                DrawStyle::Plus => draw(&mut pass, &add_kind, whole),
                DrawStyle::Knockout => {
                    // §11.4.6 per element: erase by shape, then deposit — strictly
                    // interleaved, or overlapping elements compose wrongly
                    // (ADR 0010 carries the algebra).
                    for i in whole {
                        draw(&mut pass, &erase_kind, i..i.saturating_add(1));
                        draw(&mut pass, &add_kind, i..i.saturating_add(1));
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

    /// Seed a non-isolated group's buffer with its backdrop (§11.4.4): one REPLACE
    /// blit of the parent's accumulated texture into the child's first texture.
    ///
    /// A blit rather than `copy_texture_to_texture` because it needs no copy usage on
    /// every internal texture in the frame, and because it is scissored by the same
    /// rule as every other pass — under a damage patch (ADR 0012) the seed is only the
    /// pixels the frame is allowed to touch. `blit.wgsl` is a `textureLoad` and a store
    /// with no blending, so between two `Rgba8Unorm` textures it is exact.
    fn seed_layer(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
        backdrop: &wgpu::TextureView,
        into: &wgpu::TextureView,
    ) {
        let bind = self.device.blit_bind(backdrop);
        let (pipeline, compiled) = self
            .device
            .pipelines()
            .get(Kind::Blit, wgpu::TextureFormat::Rgba8Unorm);
        if let Some(duration) = compiled {
            self.phases.push(("pipeline compile (first use)", duration));
        }
        let stamp = self.pass_stamp();
        let mut pass = recorder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("quorra seed non-isolated group"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: into,
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
    let (over, erase, add) = match batch.kind {
        BatchKind::Rect => (Kind::RectOver, Kind::RectErase, Kind::RectAdd),
        BatchKind::Quad => (Kind::CoverOver, Kind::CoverErase, Kind::CoverAdd),
    };
    style_kinds(batch.style, over, erase, add)
        .into_iter()
        .flatten()
        .collect()
}

/// The pipelines one style needs, in the order they must run: over alone, the knockout
/// erase/add pair in ADR 0010's strict order, or one half of that pair on its own when
/// the scene asked for §11.4.6's stages by name (ADR 0025).
fn style_kinds(style: DrawStyle, over: Kind, erase: Kind, add: Kind) -> [Option<Kind>; 2] {
    match style {
        DrawStyle::Over => [Some(over), None],
        DrawStyle::Knockout => [Some(erase), Some(add)],
        DrawStyle::DestOut => [Some(erase), None],
        DrawStyle::Plus => [Some(add), None],
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

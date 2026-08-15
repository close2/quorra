//! The content pass: the marks themselves, drawn onto one attachment.
//!
//! A run of consecutive drawable ops becomes exactly one render pass, which is why the
//! preparation is a phase of its own: every bind group and every pipeline the run needs
//! has to exist *before* the pass borrows the recorder, and none of them can be built
//! while it does.
//!
//! Knockout batches run their erase/add pair strictly per element (ADR 0010):
//! interleaving is what makes overlapping knockout elements compose per ISO 32000-2
//! §11.4.6 rather than approximately. Everything else is instanced, because a single
//! pass over independent marks needs no interleaving at all.

use std::collections::HashMap;
use std::sync::Arc;

use crate::encode::{Batch, BatchKind, DrawStyle, FunctionOp, ImageOp, Op, ShadedOp};
use crate::error::RenderError;
use crate::pipeline::{Kind, Style};

use super::Executor;

/// What a content pass does with the pixels already in the attachment it draws onto.
///
/// The first pass onto a plan's accumulator clears it: §11.4.5 begins a group over a
/// fully transparent initial backdrop, and §3 hands the caller pixels over one. Every
/// later pass onto the same accumulator keeps what the earlier ones put there, because
/// the painter's order is passes as much as it is instances — so this says which pass
/// this is, and never a preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PassLoad {
    /// The first pass onto this attachment: clear to transparent, then draw.
    Clear,
    /// A later pass: load what is there and draw over it.
    Keep,
}

/// One drawable item of a pass: an instanced lane batch, or a single-quad op
/// (image, shading, mesh, function — ADR 0011's rare cases and ADR 0053's).
pub(crate) enum RunOp {
    Batch(Batch),
    Image(ImageOp),
    Shaded(ShadedOp),
    Function(FunctionOp),
}

/// A prepared item of a pass: everything a draw needs that cannot be made while
/// the pass borrows the recorder.
enum Ready {
    Batch(Batch),
    /// Over is one pipeline; knockout is the erase/add pair, in order.
    ///
    /// The pipelines are resolved here rather than named by [`Kind`] and looked up in the
    /// pass, because ADR 0053's generated ones have no `Kind` to name — their key is a
    /// program's content hash. Resolving both families the same way is what keeps the pass
    /// itself ignorant of which is which.
    Single {
        pipelines: [Option<Arc<wgpu::RenderPipeline>>; 2],
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
            Op::Function(function) => RunOp::Function(**function),
            // The callers collect runs of drawable ops only.
            Op::Child(_) => unreachable!("a draw run contains no child composites"),
        })
        .collect()
}

impl Executor<'_> {
    /// One render pass of lane batches and single-quad ops onto `view`. Public to
    /// the device so the flat fast path draws the root directly onto the frame's
    /// target.
    pub(crate) fn draw_pass(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        load: PassLoad,
        ops: &[RunOp],
    ) -> Result<(), RenderError> {
        let (ready, needed) = self.prepare_run(ops, format)?;
        // The attachment this pass writes is the current plan's region, and every lane
        // maps device space through it (ADR 0036).
        let globals = self.device.region_globals(self.region);
        let mut pipelines = HashMap::new();
        for kind in needed {
            let (pipeline, compiled) = self.device.pipelines().get(kind, format)?;
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
                    load: match load {
                        PassLoad::Clear => wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        PassLoad::Keep => wgpu::LoadOp::Load,
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: stamp,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.scissor_pass(&mut pass, self.region, None);
        for item in &ready {
            let batch = match item {
                Ready::Batch(batch) => batch,
                Ready::Single { pipelines, bind } => {
                    for pipeline in pipelines.iter().flatten() {
                        pass.set_pipeline(pipeline);
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
                    pass.set_bind_group(0, &globals, &[]);
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

    /// Phase one of a draw pass: build every bind group and resolve every pipeline the
    /// run needs — none of which can happen while the pass borrows the recorder.
    ///
    /// The batches hand back a list of [`Kind`]s for the pass to look up, and the
    /// single-quad ops hand back resolved pipelines. That asymmetry is the point rather
    /// than an oversight: a batch's pipeline is one of a fixed table and is shared by every
    /// batch of its kind in the run, while a function quad's is generated per program and
    /// belongs to that op alone.
    fn prepare_run(
        &mut self,
        ops: &[RunOp],
        format: wgpu::TextureFormat,
    ) -> Result<(Vec<Ready>, Vec<Kind>), RenderError> {
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
                    let mask = self.mask_for(image.mask);
                    let scratch = self.scratch_view.as_ref().unwrap_or(&self.dummy_view);
                    let bind = self.device.image_bind(image, self.region, mask, scratch)?;
                    let pipelines = self.lane_pipelines(kinds, format)?;
                    ready.push(Ready::Single { pipelines, bind });
                }
                RunOp::Shaded(shaded) => {
                    let kinds = style_kinds(
                        shaded.style,
                        Kind::ShadedOver,
                        Kind::ShadedErase,
                        Kind::ShadedAdd,
                    );
                    let mask = self.mask_for(shaded.mask);
                    let scratch = self.scratch_view.as_ref().unwrap_or(&self.dummy_view);
                    let bind = self
                        .device
                        .shaded_bind(shaded, self.region, scratch, mask)?;
                    let pipelines = self.lane_pipelines(kinds, format)?;
                    ready.push(Ready::Single { pipelines, bind });
                }
                RunOp::Function(function) => {
                    let mask = self.mask_for(function.mask());
                    let scratch = self.scratch_view.as_ref().unwrap_or(&self.dummy_view);
                    let bind = self.function_bind(function, scratch, mask);
                    let pipelines = self.function_pipelines(function, format)?;
                    ready.push(Ready::Single { pipelines, bind });
                }
            }
        }
        Ok((ready, needed))
    }

    /// The pipelines one fixed-table single-quad op draws with, compiling on first use.
    fn lane_pipelines(
        &mut self,
        kinds: [Option<Kind>; 2],
        format: wgpu::TextureFormat,
    ) -> Result<[Option<Arc<wgpu::RenderPipeline>>; 2], RenderError> {
        let mut resolved = [None, None];
        for (slot, kind) in resolved.iter_mut().zip(kinds) {
            let Some(kind) = kind else { continue };
            let (pipeline, compiled) = self.device.pipelines().get(kind, format)?;
            if let Some(duration) = compiled {
                self.phases.push(("pipeline compile (first use)", duration));
            }
            *slot = Some(pipeline);
        }
        Ok(resolved)
    }

    /// Build (once per mask) the lane bind group carrying atlas, scratch, and the mask's
    /// view together with where it sits.
    ///
    /// Keyed by the mask, which is what the placement is a property of — a batch changes
    /// mask, the region it draws into does not (that is `Globals`, group 0).
    fn ensure_lane_bind(&mut self, mask: Option<u32>) {
        if self.lane_binds.contains_key(&mask) {
            return;
        }
        let (mask_view, placement) = self.mask_for(mask);
        let atlas = self.atlas_view.as_ref().unwrap_or(&self.dummy_view);
        let scratch = self.scratch_view.as_ref().unwrap_or(&self.dummy_view);
        let bind = self.device.lane_bind(atlas, scratch, mask_view, placement);
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

/// The pipelines one style needs, in the order they must run — one lane family's [`Kind`]s
/// under [`Style::of`], which is where that rule is stated for all five families.
fn style_kinds(style: DrawStyle, over: Kind, erase: Kind, add: Kind) -> [Option<Kind>; 2] {
    Style::of(style).map(|wanted| {
        wanted.map(|wanted| match wanted {
            Style::Over => over,
            Style::Erase => erase,
            Style::Add => add,
        })
    })
}

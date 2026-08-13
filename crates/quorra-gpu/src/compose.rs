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
use crate::layers::LayerPool;
use crate::mask::{MaskPlacement, Realised};
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
    /// Realised masks by mask index: the R8 texture and where it sits (ADR 0037).
    pub masks: Vec<Option<Realised>>,
    /// Lane instance buffers.
    pub rect_buffer: Option<wgpu::Buffer>,
    pub quad_buffer: Option<wgpu::Buffer>,
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
    /// The region of the frame the pass being recorded renders into (ADR 0036): the
    /// whole target while the root is drawing, a plan's own rectangle inside a group.
    pub region: Region,
}

/// One plan's finished pixels — the texture itself rather than a view, because the
/// caller is what gives it back to the pool (ADR 0020).
pub(crate) struct Rendered {
    texture: wgpu::Texture,
    /// Where in device space this plan's texture sits, which the composite that reads it
    /// must subtract (ADR 0036).
    region: Region,
}

/// A plan's rectangle of the frame: where its texture starts and how big it is.
///
/// Whole pixels, clamped to the target — a layer never holds anything outside the frame,
/// because every lane already draws into `bounds ∩ clip ∩ target` and no further.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    /// The whole target, which is what the root plan renders into.
    pub(crate) const fn whole(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// A device-space rectangle in this region's own coordinates: the two intersected,
    /// then moved to the region's origin.
    ///
    /// What a pass rendering into this region must scissor to when the frame patches
    /// damage (ADR 0012): the damage bounding box is stated in device space and a
    /// scissor is stated in the attachment's, and wgpu refuses — with a validation
    /// error, not a wrong picture — a scissor that leaves the attachment. Empty when the
    /// two do not meet, which draws nothing; the pass's load op still clears, so the
    /// attachment is written whole either way (`layers.rs` relies on that for reuse).
    pub(crate) fn scissor_in(self, rect: [u32; 4]) -> [u32; 4] {
        let [x, y, width, height] = rect;
        let left = x.max(self.x);
        let top = y.max(self.y);
        let right = x
            .saturating_add(width)
            .min(self.x.saturating_add(self.width));
        let bottom = y
            .saturating_add(height)
            .min(self.y.saturating_add(self.height));
        [
            left.saturating_sub(self.x),
            top.saturating_sub(self.y),
            right.saturating_sub(left),
            bottom.saturating_sub(top),
        ]
    }

    /// This region as a device-space rectangle.
    pub(crate) const fn rect(self) -> [u32; 4] {
        [self.x, self.y, self.width, self.height]
    }

    /// The rectangle two regions share, or `None` when they do not meet.
    ///
    /// A child's region and its parent's need not contain one another: a plan's bounds
    /// grow by each child's bounds **intersected with the clip the composite will apply**
    /// (`encode.rs`), so a child clipped down to a corner has a region larger than the
    /// part of the parent it can reach. Their meeting is what the composite writes.
    ///
    /// `None` is a child with no part of its parent to write, which composites to
    /// nothing. The encoder emits no child that its clip empties (ADR 0041), so what is
    /// left here is what the two roundings can produce on their own — this rounds pixel
    /// regions out and clamps them to the target, where the encoder tests device-space
    /// rectangles — and the answer the arithmetic gives for it is the same one:
    /// `clip_coverage` is zero everywhere such a child could have contributed.
    pub(crate) fn meet(self, other: Self) -> Option<Self> {
        let [x, y, width, height] = self.scissor_in(other.rect());
        (width > 0 && height > 0).then_some(Self {
            x: self.x.saturating_add(x),
            y: self.y.saturating_add(y),
            width,
            height,
        })
    }

    /// The pixels a plan's device bounds cover, rounded out and clamped.
    ///
    /// A plan that marks nothing gets one texel rather than none: wgpu refuses a
    /// zero-sized texture, and a composite still reads whatever the plan left — which
    /// for an empty plan is a cleared texel that contributes nothing.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped below
    pub(crate) fn of(bounds: Option<[f32; 4]>, width: u32, height: u32) -> Self {
        let Some([x0, y0, x1, y1]) = bounds else {
            return Self {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            };
        };
        let left = x0.floor().max(0.0).min(f32::from(u16::MAX)) as u32;
        let top = y0.floor().max(0.0).min(f32::from(u16::MAX)) as u32;
        let right = (x1.ceil().max(0.0).min(f32::from(u16::MAX)) as u32).min(width);
        let bottom = (y1.ceil().max(0.0).min(f32::from(u16::MAX)) as u32).min(height);
        Self {
            x: left.min(width.saturating_sub(1)),
            y: top.min(height.saturating_sub(1)),
            width: right.saturating_sub(left).max(1),
            height: bottom.saturating_sub(top).max(1),
        }
    }
}

impl Rendered {
    /// The texture holding this plan's result.
    pub(crate) fn view(&self) -> wgpu::TextureView {
        view_of(&self.texture)
    }

    /// Where in device space that texture sits.
    pub(crate) const fn region(&self) -> Region {
        self.region
    }
}

/// The default view of a whole texture, which is the only kind this crate makes.
fn view_of(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// A region's extent, as the shader's floats.
#[allow(clippy::cast_precision_loss)] // extents are exact in f32
fn extent(region: Region) -> [f32; 2] {
    [region.width as f32, region.height as f32]
}

/// The source origin a pass writing the whole target reads the root at: negative, because
/// the root's texel (0, 0) is the *device* pixel `region.x, region.y` (ADR 0039).
#[allow(clippy::cast_precision_loss)] // extents are exact in f32
fn from_root(region: Region) -> [f32; 2] {
    [-(region.x as f32), -(region.y as f32)]
}

/// The rectangle two scissor rectangles share, in the coordinates both are stated in.
fn overlap(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    let left = a[0].max(b[0]);
    let top = a[1].max(b[1]);
    let right = a[0].saturating_add(a[2]).min(b[0].saturating_add(b[2]));
    let bottom = a[1].saturating_add(a[3]).min(b[1].saturating_add(b[3]));
    [
        left,
        top,
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    ]
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
            // A soft mask's group renders on its own, onto transparency (§11.5), at its
            // own plan's rectangle like every other layer (ADR 0037).
            //
            // **The reduce below needs no origin**: it reads the group at the fragment's
            // own position and writes the R8 at the same one, and the two textures are
            // the same size, so they map 1:1 wherever that rectangle sits. What the
            // frame's five sampling sites need is where the R8 *is* and what surrounds
            // it, which is the placement below.
            let group = self.render_plan(recorder, plan.root.saturating_add(1), None)?;
            let region = group.region;
            let group_view = group.view();
            let mask_texture = self.device.create_internal_texture(
                "quorra soft mask",
                region.width,
                region.height,
                wgpu::TextureFormat::R8Unorm,
            );
            let mask_view = mask_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind = self.device.reduce_bind(plan, &group_view);
            let (pipeline, compiled) = self
                .device
                .pipelines()
                .get(Kind::Reduce, wgpu::TextureFormat::R8Unorm)?;
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
            self.scissor_pass(&mut pass, region, None);
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            // The reduce has read it; the mask's own R8 is what outlives this loop.
            self.pool.release(group.texture);
            if let Some(slot) = self.masks.get_mut(index) {
                *slot = Some(Realised {
                    view: mask_view,
                    placement: MaskPlacement {
                        #[allow(clippy::cast_precision_loss)] // extents are exact in f32
                        origin: [region.x as f32, region.y as f32],
                        #[allow(clippy::cast_precision_loss)]
                        size: [region.width as f32, region.height as f32],
                        outside: crate::mask::transparent_value(plan),
                    },
                });
            }
        }
        Ok(())
    }

    /// The mask an op names: the view to bind, and where its texels are.
    ///
    /// An op that names no mask — or one the frame did not realise — gets the 1 × 1
    /// stand-in and [`MaskPlacement::ABSENT`], whose every sample is outside it and so
    /// admits everything. The view still has to be bound: a layout entry is not optional.
    fn mask_for(&self, mask: Option<u32>) -> (&wgpu::TextureView, MaskPlacement) {
        let realised = mask
            .and_then(|index| self.masks.get(index as usize))
            .and_then(Option::as_ref);
        realised.map_or((&self.dummy_view, MaskPlacement::ABSENT), |realised| {
            (&realised.view, realised.placement)
        })
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
        // **As big as the plan, not as big as the target** (ADR 0036) — the root
        // included, since ADR 0039 measured what roots actually mark. One exception is
        // left: a plan that is seeded takes its parent's region, because §11.4.4's initial
        // backdrop is copied in texel for texel and the interpolation that later takes it
        // back out (ADR 0019) is stated over the whole of the group's own buffer.
        //
        // A plan that marks nothing still needs somewhere for the composite to read, and
        // one texel is enough for a rectangle nobody samples.
        let region = if seed.is_some() {
            self.region
        } else {
            Region::of(plan.bounds, self.width, self.height)
        };
        // One texture, not a ping-pong pair (ADR 0038): a child's composite writes into
        // this same accumulator, because the pixels it needs to read are copied out of it
        // first — at the child's size, which is all the composite writes.
        let accumulator = self.pool.acquire(self.device, region.width, region.height);
        let view = view_of(&accumulator);
        let outer = std::mem::replace(&mut self.region, region);
        let mut cleared = false;
        if let Some(backdrop) = seed {
            // The parent's region and the child's are the same rectangle for a seeded
            // plan, so this copy is texel for texel and reads nothing outside.
            self.copy_pass(
                recorder,
                "quorra seed non-isolated group",
                (backdrop, region),
                (&view, region),
                [0.0, 0.0],
            )?;
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
                        // The composite reads the accumulator through a copy, so it must
                        // exist even if nothing was drawn yet: clear it with an empty pass.
                        self.draw_pass(
                            recorder,
                            &view,
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
                    let seed = (!child_op.isolated).then_some(&view);
                    let child =
                        self.render_plan(recorder, child_op.layer.saturating_add(1), seed)?;
                    self.composite_child(recorder, &view, region, &child, &child_op)?;
                    // Every pass that reads the child has been recorded; a sibling may
                    // have its texture now.
                    self.pool.release(child.texture);
                }
            }
        }
        if !cleared {
            self.draw_pass(recorder, &view, wgpu::TextureFormat::Rgba8Unorm, true, &[])?;
        }
        self.region = outer;
        Ok(Rendered {
            texture: accumulator,
            region,
        })
    }

    /// Composite one finished child onto the plan accumulating it (§11.3.6).
    ///
    /// Two passes, and the first is what lets there be one texture per plan rather than
    /// two (ADR 0038): the composite cannot read the attachment it writes, so the pixels
    /// it is about to cover are copied out — **at the size of `child ∩ parent`**, because
    /// that is the whole of what it writes. Outside the child's own rectangle every branch
    /// of `composite.wgsl` collapses to the backdrop it read, so those pixels are already
    /// what the pass would put there.
    ///
    /// A child that meets its parent nowhere composites to nothing: the clip that shrank
    /// the parent's bounds is the same clip whose coverage the pass would multiply by, and
    /// it is zero everywhere the child could have contributed. **By the time a child is
    /// rendered it is too late to save anything by discovering that** — the encoder drops
    /// such a child before it becomes an op at all (ADR 0041), which is where the clip
    /// that emptied it is known.
    fn composite_child(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
        accumulator: &wgpu::TextureView,
        region: Region,
        child: &Rendered,
        op: &ChildOp,
    ) -> Result<(), RenderError> {
        let Some(onto) = region.meet(child.region) else {
            return Ok(());
        };
        let copy = self.pool.acquire(self.device, onto.width, onto.height);
        let copy_view = view_of(&copy);
        #[allow(clippy::cast_precision_loss)] // extents are exact in f32
        let from = [
            onto.x.saturating_sub(region.x) as f32,
            onto.y.saturating_sub(region.y) as f32,
        ];
        self.copy_pass(
            recorder,
            "quorra composite backdrop",
            (accumulator, region),
            (&copy_view, onto),
            from,
        )?;
        self.composite_pass(
            recorder,
            accumulator,
            region,
            (&copy_view, onto),
            (&child.view(), child.region),
            op,
        )?;
        self.pool.release(copy);
        Ok(())
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
        self.scissor_pass(&mut pass, self.region, None);
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
                    let mask = self.mask_for(image.mask);
                    let scratch = self.scratch_view.as_ref().unwrap_or(&self.dummy_view);
                    let bind = self.device.image_bind(image, self.region, mask, scratch)?;
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
                    let mask = self.mask_for(shaded.mask);
                    let scratch = self.scratch_view.as_ref().unwrap_or(&self.dummy_view);
                    let bind = self
                        .device
                        .shaded_bind(shaded, self.region, scratch, mask)?;
                    ready.push(Ready::Single { kinds, bind });
                }
            }
        }
        Ok((ready, needed))
    }

    /// One composite pass: `accumulator = child over/blended-onto backdrop` per §11.3.6.
    ///
    /// The attachment is the plan's own accumulator and the pass is **scissored to
    /// `onto`** — the part of it the child can reach (ADR 0038). `backdrop` holds the copy
    /// of exactly those pixels, made before this pass because a pass cannot read what it
    /// writes. The load op is `Load` for the same reason: everything outside the scissor
    /// is already what this pass would have written there.
    fn composite_pass(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
        accumulator: &wgpu::TextureView,
        region: Region,
        backdrop: (&wgpu::TextureView, Region),
        child: (&wgpu::TextureView, Region),
        op: &ChildOp,
    ) -> Result<(), RenderError> {
        let mask = self.mask_for(op.mask);
        let scratch = self.scratch_view.as_ref().unwrap_or(&self.dummy_view);
        let bind = self
            .device
            .composite_bind(op, region, backdrop, child, mask, scratch);
        let (pipeline, compiled) = self
            .device
            .pipelines()
            .get(Kind::Composite, wgpu::TextureFormat::Rgba8Unorm)?;
        if let Some(duration) = compiled {
            self.phases.push(("pipeline compile (first use)", duration));
        }
        let stamp = self.pass_stamp();
        let mut pass = recorder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("quorra composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: accumulator,
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
        self.scissor_pass(&mut pass, region, Some(backdrop.1));
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }

    /// One rectangle of one texture copied into another, whole and unchanged.
    ///
    /// Two callers. §11.4.4's **seed**: a non-isolated group's buffer begins as a copy of
    /// what is under it (ADR 0019), at the parent's own origin. And the **backdrop** a
    /// composite is about to cover, read from `from` in the accumulator because the copy
    /// is the child's size rather than the plan's (ADR 0038). Both read inside the source
    /// by construction; `src_region` is what tells the shader so.
    ///
    /// A blit rather than `copy_texture_to_texture` because it needs no copy usage on
    /// every internal texture in the frame, and because it is scissored by the same rule
    /// as every other pass — under a damage patch (ADR 0012) it copies only the pixels the
    /// frame is allowed to touch. `blit.wgsl` is a `textureLoad` and a store with no
    /// blending, so between two `Rgba8Unorm` textures it is exact.
    fn copy_pass(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
        label: &str,
        src: (&wgpu::TextureView, Region),
        into: (&wgpu::TextureView, Region),
        at: [f32; 2],
    ) -> Result<(), RenderError> {
        let (src, src_region) = src;
        let (into, into_region) = into;
        let bind = self.device.blit_bind(src, at, extent(src_region));
        let (pipeline, compiled) = self
            .device
            .pipelines()
            .get(Kind::Blit, wgpu::TextureFormat::Rgba8Unorm)?;
        if let Some(duration) = compiled {
            self.phases.push(("pipeline compile (first use)", duration));
        }
        let stamp = self.pass_stamp();
        let mut pass = recorder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
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
        self.scissor_pass(&mut pass, into_region, None);
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }

    /// Blit the finished root onto the frame's target.
    ///
    /// The root is as big as what the page marks (ADR 0039) and the target is the target,
    /// so this is the one copy whose destination is larger than its source: the shader
    /// reads at `p − root.origin` and writes transparency outside the root's rectangle,
    /// which is what a page rendered onto transparency (§3) has there.
    pub(crate) fn blit_to_target(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
        src: &wgpu::TextureView,
        src_region: Region,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) -> Result<(), RenderError> {
        let bind = self
            .device
            .blit_bind(src, from_root(src_region), extent(src_region));
        let (pipeline, compiled) = self.device.pipelines().get(Kind::Blit, format)?;
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
        Ok(())
    }

    /// Patch the finished root onto the target: one scissored REPLACE blit per
    /// damage rectangle, over the target's retained contents (`LoadOp::Load`).
    /// Nothing outside the rectangles is written — that is the whole contract.
    ///
    /// Inside a rectangle but outside the root, the blit writes transparency (ADR 0039):
    /// the contract is that a patched rectangle equals a full redraw, and a full redraw
    /// leaves transparency where the page marked nothing.
    pub(crate) fn patch_to_target(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
        src: &wgpu::TextureView,
        src_region: Region,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        rects: &[[u32; 4]],
    ) -> Result<(), RenderError> {
        let bind = self
            .device
            .blit_bind(src, from_root(src_region), extent(src_region));
        let (pipeline, compiled) = self.device.pipelines().get(Kind::Blit, format)?;
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
        Ok(())
    }

    /// Scissor an internal pass: to the damage bounding box when this frame patches, and
    /// to `limit` when the pass writes only part of its attachment.
    ///
    /// `into` is the region of the frame the pass's attachment holds, because that is the
    /// space a scissor is stated in while both of the others are stated in device space
    /// (ADR 0036 made the two differ). A pass rendering into a plan smaller than the
    /// damage box is a wgpu validation error otherwise, and that is a panic inside a
    /// library rather than a refusal.
    ///
    /// `limit` is the composite's `child ∩ parent` (ADR 0038), and is the reason this
    /// takes two rectangles rather than one: a patched frame compositing a small group
    /// must honour both, and only their overlap does.
    fn scissor_pass(&self, pass: &mut wgpu::RenderPass<'_>, into: Region, limit: Option<Region>) {
        if self.scissor.is_none() && limit.is_none() {
            return;
        }
        let mut rect = [0, 0, into.width, into.height];
        if let Some(damage) = self.scissor {
            rect = overlap(rect, into.scissor_in(damage));
        }
        if let Some(limit) = limit {
            rect = overlap(rect, into.scissor_in(limit.rect()));
        }
        pass.set_scissor_rect(rect[0], rect[1], rect[2], rect[3]);
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

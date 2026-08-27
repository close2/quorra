//! The stroke arm: a width that has already been resolved, expanded into a fill.
//!
//! ISO 32000-2 §8.4.3's stroke reaches us with its device width already decided — §4.5
//! of the brief settles that upstream, and we do not re-take it — so what is left here
//! is caps, joins and miters, and then the coverage lanes a fill would have taken
//! anyway. Two things make it its own arm rather than a call into [`super::fill`]:
//!
//! - **the reach.** A stroke marks outside its outline's hull, by half the width times
//!   the miter limit (§8.4.3.5), and every visibility test in this file is that grown
//!   box rather than the outline's. Getting it wrong drops a mark that would have shown.
//! - **the order.** Visibility is decided before §11.3.5's implicit group, so a stroke
//!   outside the target costs neither its expansion nor the wrapper — which is the
//!   expensive half of an off-screen blended stroke.

use quorra_scene::{Affine, BlendMode, ClipId, Command, MaskId, OutlineId, Paint};

use super::device_space::compose;
use super::parallel::{Draw, Job};
use super::{ChildOp, DrawStyle, Encoder, Op};
use crate::error::RenderError;
use crate::raster::{self, Rule};

impl Encoder<'_> {
    /// The stroke arm: expansion via the path lane, non-Normal blends through an
    /// implicit child (as in `encode_fill`).
    #[allow(clippy::too_many_arguments)] // one command's fields, destructured once
    pub(super) fn encode_stroke(
        &mut self,
        index: usize,
        outline: OutlineId,
        transform: Affine,
        stroke: quorra_scene::Stroke,
        paint: Paint,
        clip: Option<ClipId>,
        blend: BlendMode,
        mask: Option<MaskId>,
    ) -> Result<(), RenderError> {
        // A stroke expands in device space per viewport; the replay re-dispatches the
        // whole command, in order (`replay.rs`).
        self.record_slow();
        let mask = self.use_mask(mask)?;
        let stored = self
            .resources
            .outline(outline)
            .ok_or(RenderError::UnknownOutline { outline })?;
        let to_device = compose(transform, self.viewport);
        let resolved = self.resolve_clip(clip)?;
        // The device width, resolved here — §8.4.3.2's thinnest line and §10.7.5's
        // adjustment applied where the composed transform is known (ADR 0085) — so
        // one scene states one stroke and is true at every magnification.
        let device_width = raster::resolve_width(stroke, to_device);
        // Visibility before the blend wrap, so a stroke outside the target costs
        // neither its expansion nor the implicit group §11.3.5 would put it in. The
        // outline's hull grows by the stroke's own reach: a miter join may carry a
        // corner half the width times the limit away from it (§8.4.3.5), and a cap
        // extends half the width — which a limit of at least 1 already covers.
        let reach = device_width * 0.5 * stroke.miter_limit;
        if let Some((x0, y0, x1, y1)) = self.hulls.bounds(outline, &stored.segments, &to_device)
            && self.culled((x0 - reach, y0 - reach, x1 + reach, y1 + reach), &resolved)
        {
            return Ok(());
        }
        if blend != BlendMode::Normal && self.style == DrawStyle::Over {
            // §11.3.5 for a single element: an implicit one-element group (the same
            // degeneracy argument as in `encode_fill` skips it under knockout).
            let child = self.plan_child(|encoder| {
                let plain = Command::Stroke {
                    outline,
                    transform,
                    stroke,
                    paint,
                    clip,
                    blend: BlendMode::Normal,
                    mask: None,
                };
                encoder.command(index, &plain)
            })?;
            self.push_op(Op::Child(ChildOp::implicit_blend_group(child, blend, mask)))?;
            return Ok(());
        }
        self.distinct_outlines.insert(outline.0);
        self.segments = self.segments.saturating_add(stored.segments.len() as u64);
        // A solid stroke is expansion and a fill, both of them pure functions of this
        // command's own outline, so it takes the same seam a fill does (`parallel`).
        if let Paint::Solid(color) = paint
            && let Some(rect) = self.deferrable_bounds(&resolved)
        {
            // The hull grown by the stroke's own reach, which is the box the cull above
            // already tested and so an upper bound on the expansion's device bounds.
            let bound = self
                .hulls
                .bounds(outline, &stored.segments, &to_device)
                .map_or(0, |(x0, y0, x1, y1)| {
                    self.tile_bound((x0 - reach, y0 - reach, x1 + reach, y1 + reach), &resolved)
                });
            return self.enqueue(Job::sheet(
                &stored.segments,
                to_device,
                Some(stroke),
                Rule::NonZero,
                rect,
                bound,
                Draw::new(color, resolved.rect, self.style, mask),
            ));
        }
        // Flatten under the full transform, then expand: the width arrived
        // resolved (§4.5), so our job is caps, joins and miters only.
        let span = self.clock.start();
        let polylines = raster::flatten(&stored.segments, to_device);
        let stroked = raster::stroke_polylines(&polylines, stroke, device_width);
        self.clock.geometry(span);
        match paint {
            Paint::Solid(color) => {
                // The width goes with the expansion, and it is the whole reason a stroke
                // reaches the lane chooser differently from a fill: a rule at 45° has a
                // device box far wider than the mark, and its own width is the only bound
                // on how thin it is (ADR 0070).
                self.push_coverage(
                    &stroked,
                    Rule::NonZero,
                    color,
                    &resolved,
                    Some(device_width),
                    mask,
                )
            }
            // Every non-solid paint is one quad over the stroke's coverage: which one it
            // is decided by `rare_paint`, and nothing about the difference reaches here.
            Paint::Shading { .. } | Paint::Mesh(_) | Paint::Function { .. } => {
                let Some(rare) = self.rare_paint(paint)? else {
                    return Ok(());
                };
                let style = self.style;
                self.push_rare_coverage(rare, &stroked, Rule::NonZero, &resolved, style, mask)
            }
        }
    }
}

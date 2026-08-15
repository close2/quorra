//! The plan a layer is: what it draws, in order, and the box it needs to draw it in.
//!
//! One [`LayerPlan`] per node of the frame's layer tree (ISO 32000-2 §11.4.5) — the root
//! page, and one per group the walk made a child. A plan is an ordered list of [`Op`]s
//! and a device-space box, and the two are written together on purpose: every append
//! grows the bounds by the rectangle the op will actually mark, so a plan's texture can
//! be sized to what it draws rather than to the target (ADR 0036). A site that appended
//! without marking would give a plan a texture too small for its own content, and the
//! mark would come back *clipped* — a plausible-looking wrong page rather than an error,
//! which is the outcome the brief's §5 exists to forbid.
//!
//! What a plan's ops *mean* is not here. This module is the list and the box.

use super::{Batch, ChildOp, Encoder, FunctionOp, ImageOp, ShadedOp};
use crate::error::RenderError;

/// One node of the frame's layer tree: what to draw, in order, including child
/// layers to composite at their place in the order.
#[derive(Debug, Default)]
pub(crate) struct LayerPlan {
    pub ops: Vec<Op>,
    /// Device-space union of everything this plan draws, including its children after
    /// their own clips — `None` for a plan that marks nothing (ADR 0036).
    ///
    /// What the compositor allocates for it, rather than the whole target: on the three
    /// corpus pages that refuse for bytes at 4× the plans cover 0.0 %, 0.1–0.4 % and
    /// 4–6.5 % of the page. The root is the exception and is always the target's size,
    /// because it *is* the target.
    pub bounds: Option<[f32; 4]>,
}

impl LayerPlan {
    /// Grow the plan's bounds to hold `rect`, which is already in device space.
    pub(super) fn mark(&mut self, rect: [f32; 4]) {
        if !(rect[2] > rect[0] && rect[3] > rect[1]) {
            return; // a mark with no area moves nothing
        }
        self.bounds = Some(match self.bounds {
            None => rect,
            Some(b) => [
                b[0].min(rect[0]),
                b[1].min(rect[1]),
                b[2].max(rect[2]),
                b[3].max(rect[3]),
            ],
        });
    }
}

#[derive(Debug)]
pub(crate) enum Op {
    Draw(Batch),
    /// One image quad (boxed: rare on real pages, and `Op` stays small for the
    /// common draws).
    Image(Box<ImageOp>),
    /// One shading or mesh quad.
    Shaded(Box<ShadedOp>),
    /// One quad painted by a §7.10.5 program the device evaluates (ADR 0053).
    Function(Box<FunctionOp>),
    Child(ChildOp),
}

impl Encoder<'_> {
    /// The plan currently under construction.
    pub(super) fn plan_mut(&mut self) -> &mut LayerPlan {
        if self.current_plan == usize::MAX {
            &mut self.root
        } else {
            &mut self.layers[self.current_plan]
        }
    }

    /// Append an op to the current plan, and grow the plan's bounds to hold it
    /// (ADR 0036).
    ///
    /// Here rather than at the four call sites, because a site that forgot would give a
    /// plan a texture too small for what it draws — and the mark would be *clipped*,
    /// which is a plausible-looking wrong page rather than an error. A `Draw` is the
    /// exception and marks as its instances are pushed: a batch is a range, and the
    /// rectangles are the instances'.
    /// [`Encoder::append_op`] with the queue drained first: op order is draw order, so
    /// a mark whose coverage is still pending belongs before whatever this op is
    /// (`parallel`). A batch is the exception and goes straight to `append_op`, because
    /// the instance it names has just been written by a commit that drained already.
    pub(super) fn push_op(&mut self, op: Op) -> Result<(), RenderError> {
        self.drain_queue()?;
        self.append_op(op);
        Ok(())
    }

    pub(super) fn append_op(&mut self, op: Op) {
        match &op {
            Op::Image(image) => self.plan_mut().mark(image.dest),
            Op::Shaded(shaded) => self.plan_mut().mark(shaded.dest),
            Op::Function(function) => self.plan_mut().mark(function.placement.dest),
            // A child is the one op that may not be appended at all, so it goes through
            // the method that decides — from here, so that no call site can reach the
            // plain append and skip the decision. `ChildOp` is `Copy`.
            Op::Child(child) => return self.push_child(*child),
            Op::Draw(_) => {}
        }
        self.plan_mut().ops.push(op);
    }
}

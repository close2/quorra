//! One mark's instance bytes, and the run of consecutive marks it joins.
//!
//! Two streams, one per lane the brief's §0 says a page is mostly made of: a rectangle
//! instance is eight floats that `rect.wgsl` evaluates analytically, a coverage quad is
//! sixteen that `coverage.wgsl` weights by a tile of the frame's sheet. Both strides are
//! declared here beside the writer that fills them, because the two numbers and the
//! order of the fields are the same fact stated twice — once in Rust and once in WGSL —
//! and a writer that drifted from its stride draws a picture rather than failing.
//!
//! A [`Batch`] is what the device is actually asked to draw: a run of consecutive
//! instances in one lane, under one style and one soft mask. Runs are formed by
//! *extending* — a switch of any of the three breaks the batch and starts another — so
//! the painter's algorithm survives as the order the instances were written in, and
//! nothing here ever reorders a mark.

use quorra_scene::{Point, Rect};

use super::{Encoder, Op};
use crate::error::RenderError;

/// Bytes per rectangle instance: device rect (4 × f32), premultiplied colour
/// (4 × f32). Must match `rect.wgsl`.
pub(crate) const RECT_INSTANCE_STRIDE: u64 = 32;

/// Bytes per coverage-quad instance: dest min (2), size (2), texel origin + source
/// selector (4), premultiplied colour (4), clip rect (4) — 16 × f32. Must match
/// `coverage.wgsl`.
pub(crate) const QUAD_INSTANCE_STRIDE: u64 = 64;

/// Which lane a batch draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BatchKind {
    Rect,
    Quad,
}

/// How a batch composites: ordinary premultiplied over, or the knockout two-pass
/// (per-element erase by shape, then additive deposit — ADR 0010, §11.4.6/§4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DrawStyle {
    Over,
    Knockout,
    /// §11.4.6's first stage alone: scale the backdrop by `1 − shape` and deposit
    /// nothing ([`Compose::DestOut`](quorra_scene::Compose::DestOut), ADR 0025). The same erase pass the knockout pair
    /// opens with, asked for by name.
    DestOut,
    /// §11.4.6's second stage alone: add the mark, premultiplied ([`Compose::Plus`](quorra_scene::Compose::Plus)).
    Plus,
}

/// A run of consecutive instances in one lane with one style and one soft mask, in
/// scene order — the painter's algorithm survives switching by batch breaks, not by
/// reordering.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Batch {
    pub kind: BatchKind,
    pub first: u32,
    pub count: u32,
    pub style: DrawStyle,
    /// The soft mask sampled by these draws, as a mask index.
    pub mask: Option<u32>,
}

/// Bytes to reserve for one instance stream, from the command count the frame budget
/// was checked against.
///
/// Called only after the budget check, so the value is bounded by the caller's stated
/// `frame_budget_bytes`: this reserves what the frame has already been *charged*, never
/// more. The arithmetic is `u64` because the check is, and a product that does not fit
/// a `usize` reserves **nothing** rather than saturating — a `Vec` that grows is
/// correct and slower, while `with_capacity(usize::MAX)` is an abort.
pub(super) fn instance_reserve(commands: usize, stride: u64) -> usize {
    usize::try_from((commands as u64).saturating_mul(stride)).unwrap_or(0)
}

impl Encoder<'_> {
    /// One instance of the analytic rectangle lane.
    ///
    /// `style` is a parameter rather than `self.style` because the lane now takes fills
    /// as well as [`Command::Rect`](quorra_scene::Command::Rect)s (ADR 0047), and a fill names its own pass through
    /// [`Compose`](quorra_scene::Compose): `Src` is §11.4.6's knockout, `DestOut` and `Plus` are its two stages
    /// asked for by name (ADR 0025). A `Rect` carries no compose mode and so passes the
    /// enclosing group's style, which is what this method used to read for itself.
    pub(super) fn push_rect_instance(
        &mut self,
        rect: Rect,
        color: quorra_scene::Color,
        style: DrawStyle,
        mask: Option<u32>,
    ) -> Result<(), RenderError> {
        // Draw order is the scene's order, so anything queued draws first (`parallel`).
        self.drain_queue()?;
        self.plan_mut()
            .mark([rect.min.x, rect.min.y, rect.max.x, rect.max.y]);
        let premultiplied = [
            color.r * color.a,
            color.g * color.a,
            color.b * color.a,
            color.a,
        ];
        for value in [rect.min.x, rect.min.y, rect.max.x, rect.max.y] {
            self.rect_instances.extend_from_slice(&value.to_le_bytes());
        }
        for value in premultiplied {
            self.rect_instances.extend_from_slice(&value.to_le_bytes());
        }
        self.note_batch(BatchKind::Rect, style, mask);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // one instance layout, one writer
    pub(super) fn push_quad_instance(
        &mut self,
        dest: Point,
        width: f32,
        height: f32,
        tex_x: f32,
        tex_y: f32,
        source: f32,
        color: quorra_scene::Color,
        clip: Rect,
        style: DrawStyle,
        mask: Option<u32>,
    ) -> Result<(), RenderError> {
        // Draw order is the scene's order, so anything queued draws first (`parallel`).
        self.drain_queue()?;
        self.plan_mut()
            .mark([dest.x, dest.y, dest.x + width, dest.y + height]);
        let premultiplied = [
            color.r * color.a,
            color.g * color.a,
            color.b * color.a,
            color.a,
        ];
        let values = [
            dest.x,
            dest.y,
            width,
            height,
            tex_x,
            tex_y,
            source,
            0.0,
            premultiplied[0],
            premultiplied[1],
            premultiplied[2],
            premultiplied[3],
            clip.min.x,
            clip.min.y,
            clip.max.x,
            clip.max.y,
        ];
        for value in values {
            self.quad_instances.extend_from_slice(&value.to_le_bytes());
        }
        self.note_batch(BatchKind::Quad, style, mask);
        Ok(())
    }

    /// Extend the current batch, or start a new one on any switch of lane, style or
    /// mask — scene order is preserved by breaking batches, never by reordering.
    ///
    /// # Why the `- 1` cannot wrap, and the `as u32` cannot truncate
    ///
    /// This is private and both callers append **one whole instance** immediately before
    /// calling it, so the stream holds at least one stride and the index of its last
    /// instance is at least zero. There is no third caller and no path that notes a batch
    /// before writing one; if one were added, the subtraction would wrap to `u32::MAX` in
    /// release and the batch would name an instance the draw call does not have. The
    /// `no_batch_is_noted_before_its_instance_is_written` test states the invariant from
    /// the outside — a first mark's batch begins at index 0 — so the argument is gated
    /// rather than only written down.
    ///
    /// `as u32` is a truncation only above four billion instances in one stream, which is
    /// 137 GB of rectangle instances; `encode` refuses the frame at
    /// `Options::max_frame_bytes` (268 MiB by default) long before, having charged one
    /// rect and one quad per command up front.
    #[allow(clippy::cast_possible_truncation, clippy::arithmetic_side_effects)]
    fn note_batch(&mut self, kind: BatchKind, style: DrawStyle, mask: Option<u32>) {
        let index = match kind {
            BatchKind::Rect => (self.rect_instances.len() as u64 / RECT_INSTANCE_STRIDE) - 1,
            BatchKind::Quad => (self.quad_instances.len() as u64 / QUAD_INSTANCE_STRIDE) - 1,
        } as u32;
        if let Some(Op::Draw(last)) = self.plan_mut().ops.last_mut()
            && last.kind == kind
            && last.style == style
            && last.mask == mask
            && last.first + last.count == index
        {
            last.count += 1;
            return;
        }
        self.append_op(Op::Draw(Batch {
            kind,
            first: index,
            count: 1,
            style,
            mask,
        }));
    }
}

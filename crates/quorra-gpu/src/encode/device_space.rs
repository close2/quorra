//! Where a scene coordinate lands, and whether it lands on the target at all.
//!
//! Every lane starts here: a command's transform is composed with the viewport's once
//! ([`compose`]), points are taken through the result ([`apply`]), and the box that
//! comes back is measured against the target ([`Encoder::culled`]) before any geometry
//! is built. That order is the whole reason a frame costs what it *shows* rather than
//! what the page holds — at 20× magnification a page hands the encoder thousands of
//! commands for a window that displays tens of them (ADR 0015).
//!
//! The two constants of the arithmetic are here with it, because each is only correct
//! next to the other: [`target_rect`] has integer corners, which is what lets the
//! analytic lane *intersect* with it rather than merely test against it, and
//! [`CULL_MARGIN`] is the two pixels by which a quantised glyph phase and a `floor`/`ceil`
//! tile may reach outside a box this file measured. A cull with the wrong margin drops a
//! mark that would have shown, which is the one failure §5 of the brief calls worse than
//! a refusal.

use quorra_scene::{Affine, Point, Rect};

use super::Encoder;
use super::clips::ResolvedClip;
use crate::raster::DeviceTransform;
use crate::viewport::Viewport;

pub(super) fn compose(transform: Affine, viewport: &Viewport<'_>) -> DeviceTransform {
    let t = transform.then(viewport.transform);
    DeviceTransform {
        a: t.a,
        b: t.b,
        c: t.c,
        d: t.d,
        e: t.e,
        f: t.f,
    }
}

pub(super) fn transform_preserves_axes(t: &DeviceTransform) -> bool {
    // Exact zeros, as in `Affine::preserves_axes`: document transforms carry them.
    #[allow(clippy::float_cmp)]
    {
        (t.b == 0.0 && t.c == 0.0) || (t.a == 0.0 && t.d == 0.0)
    }
}

pub(super) fn apply(t: &DeviceTransform, p: Point) -> Point {
    Point::new(t.a * p.x + t.c * p.y + t.e, t.b * p.x + t.d * p.y + t.f)
}

/// A shape's device extent along one axis, as the tile that would hold it: the same
/// `floor`/`ceil` the rasteriser uses, so the number the atlas is asked about is the
/// number of texels it would be given.
pub(super) fn tile_side(low: f32, high: f32) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped below
    {
        (high.ceil() - low.floor())
            .max(0.0)
            .min(f32::from(u16::MAX)) as u32
    }
}

/// How far a lane may mark outside the device bounds a cull is tested against.
///
/// Two device pixels, and each one is a real mechanism rather than a safety margin:
/// the glyph lane rasterises at a *quantised* sub-pixel phase, which moves a tile by
/// under one pixel from the transform its bounds were taken from
/// ([`Encoder::push_glyph`]); and every coverage tile expands to whole pixels by
/// `floor`/`ceil`, which reaches under one pixel further again. Flattening adds
/// nothing to this — a flattened point lies on the curve, which lies inside the
/// control hull [`HullMemo::bounds`](super::hull::HullMemo::bounds) measures.
pub(super) const CULL_MARGIN: f32 = 2.0;

/// The target's own pixel rectangle — what a command has to reach to draw anything.
///
/// Its corners are integers, and that is load-bearing rather than incidental: an
/// edge landing exactly on a pixel boundary contributes full coverage to the pixel
/// inside it and none to the pixel outside, so a rectangle *clipped* to this
/// rectangle covers every pixel it covered before. That is why the analytic lane may
/// intersect its geometry with the target and not merely test against it, while a
/// clip at a fractional coordinate — a real edge, which must antialias — is the
/// intersection ADR 0007 already reasons about.
#[allow(clippy::cast_precision_loss)] // viewport extents are far below f32's exact range
pub(super) fn target_rect(viewport: &Viewport<'_>) -> Rect {
    Rect::new(
        Point::new(0.0, 0.0),
        Point::new(viewport.width as f32, viewport.height as f32),
    )
}

impl Encoder<'_> {
    /// Whether a command with these device bounds can mark no pixel, and so need
    /// never be built.
    ///
    /// Every lane already draws into `bounds ∩ clip ∩ target` and no further —
    /// `coverage_tile`, `encode_rect` and `encode_image` each intersect exactly those
    /// three. Testing it *before* the geometry exists is what makes a frame cost what
    /// it shows rather than what the page holds: at 20× magnification a page hands
    /// the encoder thousands of commands for a window that displays tens of them, and
    /// flattening the rest cost 9.35 ms of a 14.4 ms frame (ADR 0015).
    ///
    /// **Not §5's forbidden silence.** The test establishes that the command had no
    /// pixel to mark, so the frame is byte-for-byte the one that would have built the
    /// command and thrown it away — nothing is approximated and nothing is dropped
    /// that would have shown. [`Counters::commands_culled`] reports how often it
    /// fired, so the saving is measured rather than assumed.
    ///
    /// [`Counters::commands_culled`]: crate::frame::Counters::commands_culled
    /// **What it costs when it wins nothing**, since it runs once per command on the
    /// hottest walk there is: a page with nothing outside the target encodes 6% slower
    /// (5 933 commands, 0.76 → 0.81 ms; ADR 0015's table). Writing the same test on
    /// scalars instead of through [`Rect::intersection`] measured the same 6%, so the
    /// clear construction is the one that stays.
    pub(super) fn culled(&mut self, bounds: (f32, f32, f32, f32), clip: &ResolvedClip) -> bool {
        let (x0, y0, x1, y1) = bounds;
        let reach = Rect::new(
            Point::new(x0 - CULL_MARGIN, y0 - CULL_MARGIN),
            Point::new(x1 + CULL_MARGIN, y1 + CULL_MARGIN),
        );
        if reach
            .intersection(clip.rect)
            .intersection(self.visible)
            .is_empty()
        {
            self.note_culled();
            return true;
        }
        false
    }

    /// Record a command that reaches no pixel of the target.
    ///
    /// Its own method because two lanes decide visibility differently and both must
    /// count: the coverage lanes test bounds inflated by [`CULL_MARGIN`], while the
    /// analytic rectangle and image lanes intersect exactly the region they draw and
    /// so need no margin at all.
    pub(super) fn note_culled(&mut self) {
        self.culled = self.culled.saturating_add(1);
    }
}

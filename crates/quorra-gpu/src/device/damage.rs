//! How one frame treats the viewport's damage list (ADR 0012): what the rectangles
//! are read as, and which target can honour them at all.
//!
//! A damage list is a claim about what has *not* changed, and it is only believable
//! where the previous frame's pixels are still there to be kept. A `Texture` target is
//! the caller's and retains its contents, so a well-formed list is honoured exactly;
//! a `Surface` texture's previous contents are not guaranteed by the swapchain and a
//! `Readback` frame starts from a texture that did not exist a moment ago, so both
//! redraw the whole target — and say so in a [`Report`], because a frame that quietly
//! ignored the list would be exactly the plausible-looking wrong page §5 forbids.
//!
//! A malformed rectangle is refused by index rather than repaired. A well-formed one
//! that falls entirely outside the target is dropped, which is not the same thing: the
//! caller's claim was sound and simply covers no pixel.

use super::Device;
use crate::error::RenderError;
use crate::report::{Report, ReportKind};
use crate::target::Target;
use crate::viewport::Viewport;

/// How one frame treats the viewport's damage list (ADR 0012).
pub(super) enum DamagePlan {
    /// Redraw everything: empty damage, or a target with no retained contents.
    Full,
    /// Render internally, scissored to `bbox`, and patch exactly `rects` onto the
    /// caller's texture — both as `[x, y, width, height]` in target pixels.
    Patch {
        bbox: [u32; 4],
        rects: Vec<[u32; 4]>,
    },
}

impl Device {
    /// Decide how this frame treats the viewport's damage list (ADR 0012).
    ///
    /// A `Texture` target retains its contents under the caller's ownership, so a
    /// valid damage list is honoured exactly there. A `Surface` texture's previous
    /// contents are not guaranteed by the swapchain and a `Readback` frame starts
    /// from a fresh texture — neither has anything to patch, so both redraw fully
    /// and say so in a [`Report`].
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // snapped, clamped
    pub(super) fn plan_damage(
        viewport: &Viewport<'_>,
        into: &Target<'_>,
        reports: &mut Vec<Report>,
    ) -> Result<DamagePlan, RenderError> {
        if viewport.damage.is_empty() {
            return Ok(DamagePlan::Full);
        }
        for (index, rect) in viewport.damage.iter().enumerate() {
            let finite = rect.min.x.is_finite()
                && rect.min.y.is_finite()
                && rect.max.x.is_finite()
                && rect.max.y.is_finite();
            if !finite || rect.min.x > rect.max.x || rect.min.y > rect.max.y {
                return Err(RenderError::InvalidDamage { index });
            }
        }
        let kind = match into {
            Target::Texture(_) => None,
            Target::Surface => Some("Surface"),
            Target::Readback => Some("Readback"),
        };
        if let Some(kind) = kind {
            reports.push(Report {
                kind: ReportKind::DamageNotHonoured,
                detail: format!(
                    "a {kind} target has no retained contents to patch; the full {}x{} \
                     target was redrawn",
                    viewport.width, viewport.height
                ),
            });
            return Ok(DamagePlan::Full);
        }
        // Snap outward to whole pixels, clamp to the target, drop what falls
        // outside entirely.
        let mut rects = Vec::with_capacity(viewport.damage.len());
        let (mut bx0, mut by0, mut bx1, mut by1) = (u32::MAX, u32::MAX, 0_u32, 0_u32);
        for rect in viewport.damage {
            let x0 = rect.min.x.floor().max(0.0) as u32;
            let y0 = rect.min.y.floor().max(0.0) as u32;
            let x1 = (rect.max.x.ceil().max(0.0) as u32).min(viewport.width);
            let y1 = (rect.max.y.ceil().max(0.0) as u32).min(viewport.height);
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            bx0 = bx0.min(x0);
            by0 = by0.min(y0);
            bx1 = bx1.max(x1);
            by1 = by1.max(y1);
            rects.push([x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0)]);
        }
        let bbox = if rects.is_empty() {
            [0, 0, 0, 0]
        } else {
            [bx0, by0, bx1.saturating_sub(bx0), by1.saturating_sub(by0)]
        };
        Ok(DamagePlan::Patch { bbox, rects })
    }
}

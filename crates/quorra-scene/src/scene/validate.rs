//! Validation is here, and it is loud.
//!
//! The builder is the boundary that structured input from another process's parser
//! crosses, so §4.7's rule is enforced here: non-finite coordinates, unordered
//! rectangles, out-of-range colours, invalid strokes, unknown clip identifiers,
//! groups past their depth bound and coordinates beyond [`MAX_COORDINATE`] are refused
//! with a typed [`SceneError`] naming what was wrong — never clamped, repaired, or
//! turned into NaN geometry downstream.
//!
//! Every one of those refusals is a function in this module, so that "what a scene may
//! not contain" can be read in one sitting — with one exception, stated rather than
//! hidden: the depth bound is counted where the open frames are, in
//! [`SceneBuilder::group`](super::SceneBuilder::group)'s nesting, because the count it
//! refuses on *is* that stack.
//!
//! The [`Paint::Function`] arm is a submodule of its own ([`function`]): it is four
//! numbers with four different clauses behind them, and a reader after "what may a
//! function shading not be" should not have to read the rest of §4.7 to find them.

pub(crate) mod function;

use super::{GroupSpec, SceneBuilder};
use crate::blend::{BlendMode, Compose};
use crate::error::{GroupComposeReason, NonIsolatedReason, SceneError, StagedComposeReason};
use crate::geom::{Affine, Rect};
use crate::ids::{ClipId, MaskId};
use crate::mask::MaskKind;
use crate::paint::{Color, Paint, Stroke};

/// The largest coordinate magnitude a scene accepts, for rectangle corners and
/// transform coefficients alike.
///
/// ISO 32000-2 §4.7 of the brief requires very large coordinates to be refused loudly;
/// it does not name a bound, so this one is a deliberate choice of ours: 10⁹ is far
/// beyond any page geometry that can be rendered (a page is bounded by the format's own
/// media box limits, and a target by `Device::limits`), while still leaving f32
/// arithmetic on composed transforms comfortably inside the finite range.
pub const MAX_COORDINATE: f32 = 1e9;

impl SceneBuilder {
    /// A rectangle's three conditions, in the order §4.7 states them: finite, ordered,
    /// and inside [`MAX_COORDINATE`].
    pub(super) fn check_rect(rect: Rect) -> Result<(), SceneError> {
        if !rect.is_finite() {
            return Err(SceneError::NonFiniteRect(rect));
        }
        if !rect.is_ordered() {
            return Err(SceneError::UnorderedRect(rect));
        }
        let rect_magnitude = rect
            .min
            .x
            .abs()
            .max(rect.min.y.abs())
            .max(rect.max.x.abs())
            .max(rect.max.y.abs());
        if rect_magnitude > MAX_COORDINATE {
            return Err(SceneError::RectTooLarge {
                rect,
                limit: MAX_COORDINATE,
            });
        }
        Ok(())
    }

    pub(super) fn check_transform(transform: Affine) -> Result<(), SceneError> {
        if !transform.is_finite() {
            return Err(SceneError::NonFiniteTransform(transform));
        }
        if transform.max_coefficient() > MAX_COORDINATE {
            return Err(SceneError::TransformTooLarge {
                transform,
                limit: MAX_COORDINATE,
            });
        }
        Ok(())
    }

    pub(super) fn check_color(color: Color) -> Result<(), SceneError> {
        if color.is_valid() {
            Ok(())
        } else {
            Err(SceneError::InvalidColor(color))
        }
    }

    /// A paint, by the arm that can say what was wrong with it.
    ///
    /// [`Paint::Function`] carries four numbers of its own rather than one uploaded
    /// handle, so it gets one named variant per condition instead of this module's single
    /// [`SceneError::InvalidShading`]; see [`function::check_function_paint`].
    pub(super) fn check_paint(paint: Paint) -> Result<(), SceneError> {
        match paint {
            Paint::Solid(color) => Self::check_color(color),
            Paint::Function {
                domain,
                matrix,
                range,
                background,
                ..
            } => function::check_function_paint(domain, matrix, range, background),
            Paint::Shading { .. } | Paint::Mesh(_) => {
                if paint.is_valid() {
                    Ok(())
                } else {
                    Err(SceneError::InvalidShading)
                }
            }
        }
    }

    pub(super) fn check_stroke(stroke: Stroke) -> Result<(), SceneError> {
        if stroke.is_valid() {
            Ok(())
        } else {
            Err(SceneError::InvalidStroke(stroke))
        }
    }

    /// A constant alpha, in ISO 32000-2 §11.3.7.2's range:
    ///
    /// > All of the shape and opacity inputs shall have values in the range 0.0 to 1.0
    /// > (inclusive), with a default value of 1.0.
    ///
    /// One predicate for both of a scene's alphas because they are one parameter:
    /// §11.6.4.4's constant opacity is applied to an elementary object, and "the
    /// nonstroking alpha constant shall also be applied when painting a transparency
    /// group's results onto its backdrop". NaN and the infinities are refused on top of
    /// the clause's range — a PDF number is neither — and are what the finiteness test
    /// adds to `contains`, which would reject them anyway but says nothing about why.
    fn constant_alpha_is_valid(alpha: f32) -> bool {
        alpha.is_finite() && (0.0..=1.0).contains(&alpha)
    }

    /// A group's constant alpha, refused under the name of the thing that carried it.
    pub(super) fn check_group_alpha(alpha: f32) -> Result<(), SceneError> {
        if Self::constant_alpha_is_valid(alpha) {
            Ok(())
        } else {
            Err(SceneError::InvalidGroupAlpha { alpha })
        }
    }

    /// An image command's constant alpha. Same clause, same range, **different
    /// refusal**: a shared variant would send a caller with no group in the scene to
    /// read about one.
    pub(super) fn check_image_alpha(alpha: f32) -> Result<(), SceneError> {
        if Self::constant_alpha_is_valid(alpha) {
            Ok(())
        } else {
            Err(SceneError::InvalidImageAlpha { alpha })
        }
    }

    /// §11.5.3's backdrop colour is a colour like any other, and is the only value a
    /// [`MaskKind`] carries.
    pub(super) fn check_mask_kind(kind: MaskKind) -> Result<(), SceneError> {
        if let MaskKind::Luminosity { backdrop } = kind {
            Self::check_color(backdrop)?;
        }
        Ok(())
    }

    /// §11.4.6's staged operators are the caller's own expansion of the clause, so the
    /// two positions that already expand it refuse them (ADR 0025).
    pub(super) fn check_staged_compose(
        compose: Compose,
        blend: BlendMode,
    ) -> Result<(), SceneError> {
        if matches!(compose, Compose::SrcOver | Compose::Src) {
            return Ok(());
        }
        // Inside a knockout group is where §11.4.6 *puts* this pair, and refusing it
        // there is what ADR 0025 got wrong (ADR 0032): the clause weights each element
        // by its own source shape, so a staged element replaces the group's
        // erase-by-coverage for itself rather than applying it twice.
        let reason = (blend != BlendMode::Normal).then_some(StagedComposeReason::BlendNotNormal);
        match reason {
            Some(reason) => Err(SceneError::StagedComposeUnsupported { compose, reason }),
            None => Ok(()),
        }
    }

    /// What a group may be composited with (ADR 0033), in the order a reader of §11.4.6
    /// meets the conditions.
    ///
    /// Both refusals are §5's kind: the operator would be drawn somewhere the clause does
    /// not put it, and a plausible-looking wrong page is the worst outcome either project
    /// has a name for.
    pub(super) fn check_group_compose(spec: &GroupSpec) -> Result<(), SceneError> {
        let staged = matches!(spec.compose, Compose::DestOut | Compose::Plus);
        let reason = if matches!(spec.compose, Compose::Src) {
            // §11.4.6's element-with-shape-equal-to-coverage is what `knockout` means,
            // and a group cannot ask for both without saying which it meant.
            Some(GroupComposeReason::Source)
        } else if staged && spec.blend != BlendMode::Normal {
            // A blend mode composites the group by §11.3.5, which is the step the staged
            // pair replaces rather than joins.
            Some(GroupComposeReason::BlendNotNormal)
        } else if staged && !spec.isolated {
            // §11.4.4 seeds a non-isolated group's buffer with its own backdrop, so the
            // alpha this pair reads as a shape would be the backdrop's as well as the
            // group's.
            Some(GroupComposeReason::NonIsolated)
        } else {
            None
        };
        match reason {
            Some(reason) => Err(SceneError::GroupComposeUnsupported {
                compose: spec.compose,
                reason,
            }),
            None => Ok(()),
        }
    }

    /// §11.4.6's separate shape value, for the one element kind that cannot supply it
    /// (ADR 0069).
    ///
    /// The clause states the obligation directly:
    ///
    /// > The separate shape value shall be computed in any group that is subsequently
    /// > used as an element of a knockout group.
    ///
    /// and §11.3.7.2 says what that value is for a group: "The shape of a group object
    /// shall be the union (as defined in 11.3.7.3, "Result shape and opacity") of the
    /// shapes of the objects it contains". A finished group reaches a compositor as one
    /// premultiplied raster, whose alpha is the union of each element's shape *times its
    /// opacity* — so the two quantities §11.4.6 weights apart arrive as one number, and
    /// the group would be composited by §11.3.6 instead. That is a plausible-looking
    /// wrong page rather than a hole, which §5 of the brief refuses.
    ///
    /// **Two things this deliberately does not refuse**, and both are load-bearing:
    ///
    /// - **§11.4.6's own two stages.** [`Compose::DestOut`] then [`Compose::Plus`] on two
    ///   groups *is* the clause's `P' = (1 − f) × P + S`, with the shape half drawn as
    ///   content the caller knows to be opaque (ADR 0033). A caller who can state the
    ///   shape has the construction; one who cannot gets this refusal.
    /// - **A group deeper than one level.** The predicate is
    ///   [`element_of_knockout`](SceneBuilder::element_of_knockout) and not
    ///   [`inside_knockout`](SceneBuilder::inside_knockout), because §11.4.6 governs a
    ///   knockout group's *elements*: a group inside an ordinary group is composited by
    ///   §11.3.6 whatever encloses that ordinary group. Reading the transitive predicate
    ///   here would refuse the shape half's own nested groups — which the caller's
    ///   expansion of this very clause produces, since a group's stated shape is the
    ///   group of its elements' stated shapes.
    ///
    /// Nothing escapes by nesting, because there is nothing to escape: §11.4.6 reaches
    /// exactly one level, so a group deeper than that is not the construction this refusal
    /// is about — its own parent composites it by §11.3.6, correctly.
    pub(super) fn check_knockout_element_group(&self, spec: &GroupSpec) -> Result<(), SceneError> {
        if self.element_of_knockout() && matches!(spec.compose, Compose::SrcOver) {
            return Err(SceneError::KnockoutElementGroupUnsupported);
        }
        Ok(())
    }

    /// The three conditions of [`GroupSpec::isolated`], in the order a reader of the
    /// clause meets them. Checked before the body runs, so a refusal costs nothing
    /// that was built inside it.
    pub(super) fn check_isolation(&self, spec: &GroupSpec) -> Result<(), SceneError> {
        if spec.isolated {
            return Ok(());
        }
        let reason = if spec.blend != BlendMode::Normal {
            Some(NonIsolatedReason::GroupBlendNotNormal)
        } else if spec.knockout {
            Some(NonIsolatedReason::KnockoutGroup)
        } else if self.inside_knockout() {
            Some(NonIsolatedReason::InsideKnockoutGroup)
        } else {
            None
        };
        match reason {
            Some(reason) => Err(SceneError::NonIsolatedGroupUnsupported { reason }),
            None => Ok(()),
        }
    }

    pub(super) fn check_clip(&self, clip: Option<ClipId>) -> Result<(), SceneError> {
        if let Some(clip) = clip {
            let allocated = u32::try_from(self.clips.len()).unwrap_or(u32::MAX);
            if clip.0 >= allocated {
                return Err(SceneError::UnknownClip { clip, allocated });
            }
        }
        Ok(())
    }

    pub(super) fn check_mask(&self, mask: Option<MaskId>) -> Result<(), SceneError> {
        if let Some(mask) = mask {
            let defined = u32::try_from(self.masks.len()).unwrap_or(u32::MAX);
            if mask.0 >= defined {
                return Err(SceneError::UnknownMask { mask, defined });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::MAX_COORDINATE;
    use crate::blend::{BlendMode, Compose, FillRule};
    use crate::error::SceneError;
    use crate::geom::{Affine, Point, Rect};
    use crate::ids::{ClipId, ImageId, OutlineId};
    use crate::paint::{Color, LineCap, LineJoin, Paint, Stroke};
    use crate::scene::GroupSpec;
    use crate::scene::ImageFilter;
    use crate::scene::SceneBuilder;
    use crate::scene::fixtures::{black, plain_group, unit_rect};

    /// §4.7: every forbidden input is refused with the variant that names it, and
    /// nothing is appended.
    #[test]
    fn forbidden_inputs_are_refused_loudly() {
        let mut builder = SceneBuilder::new();

        let nan_rect = Rect::new(Point::new(f32::NAN, 0.0), Point::new(1.0, 1.0));
        assert!(matches!(
            builder.rect(nan_rect, Affine::IDENTITY, black(), None, None),
            Err(SceneError::NonFiniteRect(_))
        ));

        let unordered = Rect::new(Point::new(5.0, 0.0), Point::new(1.0, 1.0));
        assert!(matches!(
            builder.rect(unordered, Affine::IDENTITY, black(), None, None),
            Err(SceneError::UnorderedRect(_))
        ));

        let huge = Rect::new(Point::new(0.0, 0.0), Point::new(2e9, 1.0));
        assert!(matches!(
            builder.rect(huge, Affine::IDENTITY, black(), None, None),
            Err(SceneError::RectTooLarge { .. })
        ));

        let nan_transform = Affine {
            e: f32::NAN,
            ..Affine::IDENTITY
        };
        assert!(matches!(
            builder.rect(unit_rect(), nan_transform, black(), None, None),
            Err(SceneError::NonFiniteTransform(_))
        ));

        let huge_transform = Affine::translate(MAX_COORDINATE * 2.0, 0.0);
        assert!(matches!(
            builder.rect(unit_rect(), huge_transform, black(), None, None),
            Err(SceneError::TransformTooLarge { .. })
        ));

        let bad_color = Color::new(0.0, 0.0, 0.0, 1.5);
        assert!(matches!(
            builder.rect(unit_rect(), Affine::IDENTITY, bad_color, None, None),
            Err(SceneError::InvalidColor(_))
        ));

        let bad_stroke = Stroke {
            width: 0.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 4.0,
        };
        assert!(matches!(
            builder.stroke(
                OutlineId(0),
                Affine::IDENTITY,
                bad_stroke,
                Paint::Solid(black()),
                None,
                BlendMode::Normal,
                None,
            ),
            Err(SceneError::InvalidStroke(_))
        ));

        assert!(
            builder.finish().commands().is_empty(),
            "refused inputs must not be appended"
        );
    }

    /// One range, two names. Every value outside ISO 32000-2 §11.3.7.2's `0..=1` is
    /// refused from both calls that take a constant alpha, and each refusal names the
    /// thing that carried it — an image's alpha reported as a group's would send a
    /// caller with no group in the scene to read about one.
    #[test]
    fn a_constant_alpha_is_refused_under_the_name_of_what_carried_it() {
        let mut builder = SceneBuilder::new();
        let group = |builder: &mut SceneBuilder, alpha| {
            let spec = GroupSpec {
                alpha,
                ..plain_group()
            };
            builder.group(spec, |_| Ok(()))
        };
        let image = |builder: &mut SceneBuilder, alpha| {
            builder.image(
                ImageId(0),
                Affine::IDENTITY,
                alpha,
                ImageFilter::Nearest,
                None,
                BlendMode::Normal,
                None,
            )
        };

        // NaN and the infinities, then both sides of the interval.
        for alpha in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.001, 1.001] {
            assert!(
                matches!(
                    group(&mut builder, alpha),
                    Err(SceneError::InvalidGroupAlpha { .. })
                ),
                "a group's alpha of {alpha} must be refused as a group's"
            );
            assert!(
                matches!(
                    image(&mut builder, alpha),
                    Err(SceneError::InvalidImageAlpha { .. })
                ),
                "an image's alpha of {alpha} must be refused as an image's"
            );
        }

        // The clause's range is "(inclusive)", so both endpoints are accepted — checked
        // here so that a bound tightened by accident is a failure and not a silence.
        for alpha in [0.0, 0.5, 1.0] {
            assert!(
                group(&mut builder, alpha).is_ok(),
                "a group's alpha of {alpha} is inside the clause's range"
            );
            assert!(
                image(&mut builder, alpha).is_ok(),
                "an image's alpha of {alpha} is inside the clause's range"
            );
        }
    }

    /// Clip identifiers are scene-scoped: a foreign or future id is refused wherever
    /// it is presented — as a command's clip, as a chain's parent, or on a group.
    #[test]
    fn unknown_clips_are_refused_everywhere() {
        let mut builder = SceneBuilder::new();
        let foreign = ClipId(7);
        assert!(matches!(
            builder.fill(
                OutlineId(0),
                Affine::IDENTITY,
                FillRule::NonZero,
                Paint::Solid(black()),
                Some(foreign),
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            ),
            Err(SceneError::UnknownClip { .. })
        ));
        assert!(matches!(
            builder.clip(
                OutlineId(0),
                Affine::IDENTITY,
                FillRule::NonZero,
                Some(foreign)
            ),
            Err(SceneError::UnknownClip { .. })
        ));
        assert!(matches!(
            builder.group(
                GroupSpec {
                    clip: Some(foreign),
                    ..plain_group()
                },
                |_| Ok(()),
            ),
            Err(SceneError::UnknownClip { .. })
        ));
    }
}

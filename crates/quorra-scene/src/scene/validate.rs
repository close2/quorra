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
    /// [`Paint::Function`] is the arm that carries a whole program, so its refusal is
    /// [`FunctionPaint::check`](crate::function::FunctionPaint::check)'s — one named
    /// variant per condition, in the order that module states them — rather than this
    /// module's single [`SceneError::InvalidShading`]. The rules live beside the type
    /// because what they check is the program's well-formedness, not the scene's shape.
    pub(super) fn check_paint(paint: &Paint) -> Result<(), SceneError> {
        match paint {
            Paint::Solid(color) => Self::check_color(*color),
            Paint::Function(function) => function.check(),
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

    /// A constant alpha, `0..=1` — §11.4.5's group alpha, and the image lane's
    /// constant alpha, which is the same range checked the same way.
    pub(super) fn check_alpha(alpha: f32) -> Result<(), SceneError> {
        if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
            return Err(SceneError::InvalidGroupAlpha { alpha });
        }
        Ok(())
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
    use crate::ids::{ClipId, OutlineId};
    use crate::paint::{Color, LineCap, LineJoin, Paint, Stroke};
    use crate::scene::GroupSpec;
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

    /// A [`Paint::Function`] crosses the boundary like every other input: accepted when
    /// well-formed, refused *by the condition it broke* when not, and never appended.
    /// The refusal grounds themselves are `crate::function`'s and are tested there; what
    /// is tested here is that the boundary asks.
    #[test]
    fn a_malformed_function_paint_is_refused_at_the_boundary() {
        use crate::scene::fixtures::function_paint;

        let mut builder = SceneBuilder::new();
        let good = Paint::Function(std::sync::Arc::new(function_paint(1)));
        builder
            .fill(
                OutlineId(0),
                Affine::IDENTITY,
                FillRule::NonZero,
                good,
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .expect("a well-formed function paint is an ordinary paint");

        let looping = Paint::Function(std::sync::Arc::new(crate::function::FunctionPaint {
            program: std::sync::Arc::from([crate::function::FnOp::Jump { target: 0 }].as_slice()),
            ..function_paint(1)
        }));
        assert!(matches!(
            builder.stroke(
                OutlineId(0),
                Affine::IDENTITY,
                Stroke {
                    width: 1.0,
                    cap: LineCap::Butt,
                    join: LineJoin::Miter,
                    miter_limit: 4.0,
                },
                looping,
                None,
                BlendMode::Normal,
                None,
            ),
            Err(SceneError::BackwardFunctionJump { .. })
        ));
        assert_eq!(
            builder.finish().commands().len(),
            1,
            "the refused stroke must not be appended"
        );
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

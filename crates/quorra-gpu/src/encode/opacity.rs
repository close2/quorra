//! Whether a finished group's raster carries §11.6.4.2's **shape** or shape times
//! opacity — the one question a compositor cannot answer from the raster itself.
//!
//! ISO 32000-2 §11.3.7.1 defines `α = f × q`, and a premultiplied layer holds the
//! product. Everywhere §11.3.6 composites, that is all anyone needs; §8.5.4's clip is
//! the place it is not, because a clip intersects a *shape* and multiplying it into an
//! opacity is a different arithmetic with a different answer (ADR 0074). This module is
//! the proof obligation that separates the two cases, and nothing else: it reads a
//! group's commands and answers whether every opacity input inside is 1.0.
//!
//! It is a module of its own because it is a reading of the clause rather than a step of
//! the walk — [`super::layer`] owns what a group *is* to the encoder, and this owns one
//! fact about what its pixels will mean.

use quorra_scene::{Command, Paint};

/// Whether every opacity input inside a group's commands is 1.0 — which is exactly the
/// condition under which the group's finished raster carries §11.6.4.2's **shape**
/// rather than shape times opacity, and so may be met with its clip by §8.5.4's
/// intersection (ADR 0074).
///
/// # Why this is a proof and not a heuristic
///
/// ISO 32000-2 keeps shape and opacity in step by construction: §11.3.7.1 defines
/// `αs = fs × qs`, and §11.3.7.3's union and §11.4.6's knockout stages apply the *same*
/// recurrence to `f` and to `α`, differing only in the opacity inputs they carry. So if
/// every opacity input in a subtree is 1, then `α = f` at every step of it, and the
/// group's accumulated alpha is its shape. §11.6.4.2 supplies the base case:
///
/// > All elementary objects shall have an intrinsic opacity qj of 1.0 everywhere.
///
/// which leaves exactly three ways for an opacity below 1 to enter — §11.6.4.4's
/// constant (a paint's alpha, an image's alpha, a group's alpha), §11.6.4.3's soft mask
/// (opacity by ADR 0066, since a `Scene` carries no alpha-source flag), and a nested
/// group carrying either. This function refuses all three.
///
/// Blend modes, knockout and ADR 0033's staged operators are deliberately *not* refused:
/// none of them is an opacity input, and each applies its recurrence to shape and alpha
/// alike.
///
/// # The one thing that is not an opacity input and is refused anyway
///
/// A **non-isolated** nested group (§11.4.4). Its own raster is `E(B)` and its clip
/// therefore reaches it as a weight rather than as a set (ADR 0074), so what it
/// contributes to this group's alpha is `f × C` where its shape is `min(f, C)` — and the
/// two part exactly where this whole decision is about, a clip edge inside a pixel.
///
/// An isolated one is admitted, and the recursion is what makes that sound: this function
/// requires its body to pass the same test, which is the condition its own `encode_group`
/// evaluates, so an admitted nested group is one whose clip *was* intersected and whose
/// contributed alpha is its shape. The invariant travels with the answer rather than
/// being assumed by it.
///
/// # What it declines to prove, and why that direction is the safe one
///
/// A `false` costs nothing but the improvement: the composite falls back to the product
/// it has always used. A wrongly-`true` would paint a half-transparent group at more
/// than its alpha wherever its clip admits more, so every unknown answers `false`:
///
/// - **Any image**, whose samples carry alpha this walk cannot see.
/// - **Any paint but an opaque solid.** A ramp and a mesh live on the device, not in the
///   scene, and a function paint's `Background` is a colour with an alpha; proving those
///   opaque means reading resources this walk does not hold.
///
/// # Cost
///
/// One pass over the group's own commands, and a nested group's body is walked again by
/// its own `encode_group`. Nesting is bounded at [`MAX_GROUP_DEPTH`] (16) by the
/// builder, so the repeat is bounded by that factor and not by the page.
///
/// [`MAX_GROUP_DEPTH`]: quorra_scene::MAX_GROUP_DEPTH
pub(super) fn every_opacity_is_one(commands: &[Command]) -> bool {
    commands.iter().all(|command| match command {
        Command::Rect { color, mask, .. } => color.a >= 1.0 && mask.is_none(),
        Command::Fill { paint, mask, .. } | Command::Stroke { paint, mask, .. } => {
            mask.is_none() && matches!(paint, Paint::Solid(color) if color.a >= 1.0)
        }
        // An image's own samples are its opacity as much as its `alpha` is, and they are
        // uploaded rather than stated here.
        Command::Image { .. } => false,
        Command::Group { spec, commands } => {
            spec.alpha >= 1.0
                && spec.mask.is_none()
                && spec.isolated
                && every_opacity_is_one(commands)
        }
    })
}

/// [`every_opacity_is_one`], command kind by command kind.
///
/// The predicate is what decides whether a group's clip is applied as ISO 32000-2
/// §8.5.4's set or as a product (ADR 0074), so each test states which opacity input it
/// is about — and the ones that assert `true` are as load-bearing as the ones that
/// assert `false`: a predicate that answered `false` to everything would satisfy every
/// safety property this file has and buy nothing.
#[cfg(test)]
mod tests {
    use quorra_scene::{
        Affine, BlendMode, Color, Command, Compose, FillRule, GroupSpec, ImageFilter, ImageId,
        LineCap, LineJoin, MaskId, Paint, Point, Rect, Stroke,
    };

    use super::every_opacity_is_one;

    fn unit() -> Rect {
        Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0))
    }

    fn rect(alpha: f32, mask: Option<MaskId>) -> Command {
        Command::Rect {
            rect: unit(),
            transform: Affine::IDENTITY,
            color: Color::new(0.0, 0.0, 0.0, alpha),
            clip: None,
            mask,
        }
    }

    fn fill(paint: Paint, mask: Option<MaskId>, compose: Compose) -> Command {
        Command::Fill {
            outline: quorra_scene::OutlineId(0),
            transform: Affine::IDENTITY,
            rule: FillRule::NonZero,
            paint,
            clip: None,
            blend: BlendMode::Normal,
            compose,
            mask,
        }
    }

    fn hairline() -> Stroke {
        Stroke {
            width: 1.0,
            adjust: false,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 10.0,
        }
    }

    fn group(alpha: f32, mask: Option<MaskId>, commands: Vec<Command>) -> Command {
        nested(alpha, mask, true, commands)
    }

    fn nested(alpha: f32, mask: Option<MaskId>, isolated: bool, commands: Vec<Command>) -> Command {
        Command::Group {
            spec: GroupSpec {
                alpha,
                blend: BlendMode::Normal,
                clip: None,
                knockout: false,
                mask,
                compose: Compose::SrcOver,
                isolated,
            },
            commands,
        }
    }

    /// §11.6.4.4's constant opacity, wherever a command carries one: a paint's alpha, a
    /// group's alpha, an image's alpha. Any of them below 1.0 and the group's raster
    /// holds shape times opacity.
    #[test]
    fn a_constant_opacity_below_one_is_not_provable() {
        assert!(every_opacity_is_one(&[rect(1.0, None)]));
        assert!(!every_opacity_is_one(&[rect(0.5, None)]));

        let opaque = Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0));
        let translucent = Paint::Solid(Color::new(0.0, 0.0, 0.0, 0.99));
        assert!(every_opacity_is_one(&[fill(
            opaque,
            None,
            Compose::SrcOver
        )]));
        assert!(!every_opacity_is_one(&[fill(
            translucent,
            None,
            Compose::SrcOver
        )]));

        assert!(every_opacity_is_one(&[group(
            1.0,
            None,
            vec![rect(1.0, None)]
        )]));
        assert!(!every_opacity_is_one(&[group(
            0.5,
            None,
            vec![rect(1.0, None)]
        )]));
    }

    /// §11.6.4.3's mask is opacity (ADR 0066), so one anywhere in the subtree is an
    /// opacity input this walk cannot value — including on a nested group.
    #[test]
    fn a_soft_mask_anywhere_is_not_provable() {
        assert!(!every_opacity_is_one(&[rect(1.0, Some(MaskId(0)))]));
        assert!(!every_opacity_is_one(&[group(
            1.0,
            Some(MaskId(0)),
            vec![rect(1.0, None)]
        )]));
        assert!(!every_opacity_is_one(&[group(
            1.0,
            None,
            vec![rect(1.0, Some(MaskId(0)))]
        )]));
    }

    /// An image is declined whatever its constant alpha says, because its **samples**
    /// carry an alpha this walk never sees. The `1.0` here is the point: the one number
    /// the command does state is not the one that decides it.
    #[test]
    fn an_image_is_never_provable_because_its_samples_are_not_here() {
        assert!(!every_opacity_is_one(&[Command::Image {
            image: ImageId(0),
            transform: Affine::IDENTITY,
            alpha: 1.0,
            filter: ImageFilter::Nearest,
            clip: None,
            blend: BlendMode::Normal,
            mask: None,
        }]));
    }

    /// A ramp, a mesh and a §7.10.5 program live on the device rather than in the scene,
    /// so an opaque one cannot be told from a translucent one here. Declining them costs
    /// the improvement and never correctness.
    #[test]
    fn a_paint_whose_colours_are_uploaded_is_not_provable() {
        let shading = Paint::Shading {
            ramp: quorra_scene::RampId(0),
            kind: quorra_scene::ShadingKind::Axial {
                start: Point::new(0.0, 0.0),
                end: Point::new(1.0, 0.0),
                extend: (false, false),
            },
            transform: Affine::IDENTITY,
        };
        assert!(!every_opacity_is_one(&[fill(
            shading,
            None,
            Compose::SrcOver
        )]));
        assert!(!every_opacity_is_one(&[fill(
            Paint::Mesh(quorra_scene::MeshId(0)),
            None,
            Compose::SrcOver
        )]));
    }

    /// **Not** opacity inputs, and so not refused: §11.3.5's blend modes, §11.4.6's
    /// knockout, and ADR 0033's staged operators. Each applies its own recurrence to
    /// shape and to alpha alike, so `α = f` survives all three — and a predicate that
    /// declined them would leave the clause's own knockout construction unimproved for
    /// no reason.
    #[test]
    fn blending_knockout_and_the_staged_pair_are_not_opacity() {
        let opaque = Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0));
        assert!(every_opacity_is_one(&[fill(
            opaque,
            None,
            Compose::DestOut
        )]));

        let knockout = Command::Group {
            spec: GroupSpec {
                alpha: 1.0,
                blend: BlendMode::Multiply,
                clip: None,
                knockout: true,
                mask: None,
                compose: Compose::SrcOver,
                isolated: true,
            },
            commands: vec![rect(1.0, None)],
        };
        assert!(every_opacity_is_one(&[knockout]));
    }

    /// A nested **non-isolated** group is refused though its every opacity input is 1.0:
    /// its raster is §11.4.4's `E(B)`, so its own clip reaches it as a weight and what it
    /// contributes here is `f × C` where its shape is `min(f, C)` (ADR 0074). The isolated
    /// twin beside it is the control — without it this test would pass against a function
    /// that refused all nesting.
    #[test]
    fn a_nested_non_isolated_group_is_not_provable_though_it_is_opaque() {
        assert!(every_opacity_is_one(&[nested(
            1.0,
            None,
            true,
            vec![rect(1.0, None)]
        )]));
        assert!(!every_opacity_is_one(&[nested(
            1.0,
            None,
            false,
            vec![rect(1.0, None)]
        )]));
    }

    /// A stroke is a paint like any other here, and an empty group is provable: it
    /// marks nothing, its shape is 0 everywhere, and 0 = 0.
    #[test]
    fn a_stroke_follows_its_paint_and_an_empty_group_is_provable() {
        assert!(every_opacity_is_one(&[]));
        assert!(every_opacity_is_one(&[Command::Stroke {
            outline: quorra_scene::OutlineId(0),
            transform: Affine::IDENTITY,
            stroke: hairline(),
            paint: Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0)),
            clip: None,
            blend: BlendMode::Normal,
            mask: None,
        }]));
        assert!(!every_opacity_is_one(&[Command::Stroke {
            outline: quorra_scene::OutlineId(0),
            transform: Affine::IDENTITY,
            stroke: hairline(),
            paint: Paint::Solid(Color::new(0.0, 0.0, 0.0, 0.25)),
            clip: None,
            blend: BlendMode::Normal,
            mask: None,
        }]));
    }
}

//! Why the builder refused an input: one typed enum, and the three reasons it defers to.
//!
//! One responsibility, and `doc/RENDER_LIBRARY.md` §4.7 states it: structured input
//! reaching [`SceneBuilder`](crate::scene::SceneBuilder) from another process's parser
//! is refused *by name* — never clamped, never repaired, never turned into NaN geometry
//! for a later stage to discover. [`SceneError`] is that name, and it is the only error
//! this crate produces.
//!
//! Three of the refusals are about a *position* rather than a value: which operator a
//! group named, where a staged operator was placed, what surrounds a non-isolated
//! group. Each of those carries its own reason enum, because §5's "an `Err` that names
//! what overflowed" is not satisfied by an error a caller cannot attribute — and
//! because a variant that names nothing makes "how often does this happen?"
//! unanswerable, which is the rule `quorra-gpu`'s [`error`](../../quorra_gpu/error/index.html)
//! module states for its own enums.
//!
//! `Display` is written by hand rather than derived: this crate has no dependencies at
//! all (ADR 0001), and `thiserror` would be the first one.

use std::error::Error;
use std::fmt;

use crate::blend::Compose;
use crate::geom::{Affine, Rect};
use crate::ids::{ClipId, MaskId};
use crate::paint::{Color, Stroke};

/// Which of [`GroupSpec::isolated`](crate::scene::GroupSpec::isolated)'s three
/// conditions a non-isolated group broke.
///
/// Each names a case where §11.4.4's backdrop removal no longer cancels against the
/// composite that follows it, so the group's own alpha — Table 140's group alpha,
/// which a premultiplied raster does not hold — would be needed to draw it correctly.
/// A refusal here is §5 of the brief: a hole and a sentence beat a plausible lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonIsolatedReason {
    /// The group's own blend mode is not
    /// [`BlendMode::Normal`](crate::blend::BlendMode::Normal). The cancellation *is* the
    /// Normal blend function; under any other the identity is false by up to 0.91 of
    /// full scale.
    GroupBlendNotNormal,
    /// The group is itself a knockout group (§11.4.6): its elements composite with the
    /// initial backdrop rather than with each other, so seeding that backdrop into the
    /// accumulating buffer describes neither model.
    KnockoutGroup,
    /// The group is nested inside a knockout group, whose elements composite with
    /// *its* initial backdrop — which is not the accumulated content this group would
    /// be seeded from.
    InsideKnockoutGroup,
}

/// Why a group cannot be composited with the operator it named (ADR 0033).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupComposeReason {
    /// [`Compose::Src`] on a group:
    /// [`GroupSpec::knockout`](crate::scene::GroupSpec::knockout) is what §11.4.6 calls
    /// that, and asking for both would be asking the same question twice.
    Source,
    /// The group also carries a blend mode, which composites it by §11.3.5 — the step
    /// the staged pair replaces rather than joins.
    BlendNotNormal,
    /// The group is not isolated, so §11.4.4 seeds its buffer with its own backdrop and
    /// the alpha the erase half reads as a shape would carry the backdrop's too.
    NonIsolated,
}

impl GroupComposeReason {
    /// The clause-shaped half of the message, so `Display` stays one screen.
    fn because(self) -> &'static str {
        match self {
            Self::Source => {
                "a group whose elements each replace the backdrop is what `knockout` \
                 states (§11.4.6)"
            }
            Self::BlendNotNormal => {
                "it also carries a blend mode, which composites the group by §11.3.5"
            }
            Self::NonIsolated => {
                "it is not isolated, so §11.4.4 seeds its buffer with its own backdrop \
                 and the alpha this operator reads as a shape would carry that backdrop \
                 too"
            }
        }
    }
}

/// Why one of §11.4.6's staged operators cannot be drawn where it was placed.
///
/// [`Compose::DestOut`] and [`Compose::Plus`] are a caller's own expansion of §11.4.6's
/// second stage, and one position still refuses them because it already *is* that stage
/// by another route — a refusal rather than a guess (§5 of the brief).
///
/// **A knockout group is no longer one of them** (ADR 0032). It was, on the reading that
/// a group whose elements are staged per element cannot also have an element stage
/// itself; the clause says otherwise, weighting each element by *its own* source shape,
/// so a staged element replaces the group's erase-by-coverage for itself. That position
/// is where §11.4.6 puts the pair, and it was the only position the caller's interpreter
/// emits it from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagedComposeReason {
    /// The mark carries a blend mode other than
    /// [`BlendMode::Normal`](crate::blend::BlendMode::Normal), which wraps it in an
    /// implicit one-element group (§11.3.5) — so the operator would compose the group
    /// rather than the element, which is not where the clause puts it.
    BlendNotNormal,
}

impl StagedComposeReason {
    /// The clause-shaped half of the message; see [`GroupComposeReason::because`].
    fn because(self) -> &'static str {
        match self {
            Self::BlendNotNormal => {
                "it also carries a blend mode, which puts it in an implicit one-element \
                 group (§11.3.5)"
            }
        }
    }
}

/// Why the builder refused an input. Every variant names what was wrong with which
/// value, because §4.7's refusal is only useful to a caller if it can be attributed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SceneError {
    /// A rectangle had a NaN or infinite coordinate.
    NonFiniteRect(Rect),
    /// A rectangle's `min` exceeded its `max` on some axis. Not repaired by swapping:
    /// an unordered rectangle from a correct interpreter means something upstream went
    /// wrong, and hiding that would hide the defect.
    UnorderedRect(Rect),
    /// A rectangle coordinate exceeded
    /// [`MAX_COORDINATE`](crate::scene::MAX_COORDINATE) in magnitude.
    RectTooLarge {
        /// The offending rectangle.
        rect: Rect,
        /// The limit it exceeded, named per §5 of the brief.
        limit: f32,
    },
    /// A transform had a NaN or infinite coefficient.
    NonFiniteTransform(Affine),
    /// A transform coefficient exceeded
    /// [`MAX_COORDINATE`](crate::scene::MAX_COORDINATE) in magnitude.
    TransformTooLarge {
        /// The offending transform.
        transform: Affine,
        /// The limit it exceeded.
        limit: f32,
    },
    /// A colour component was NaN, infinite, or outside `0..=1`. Not clamped: see
    /// [`Color::is_valid`].
    InvalidColor(Color),
    /// A shading's geometry was non-finite, out of the coordinate bound, or carried
    /// a negative radius ([`crate::paint::ShadingKind::is_valid`]).
    InvalidShading,
    /// A stroke violated [`Stroke::is_valid`] — a non-positive or non-finite width
    /// (widths arrive resolved and positive, §4.5), or a miter limit below 1.
    InvalidStroke(Stroke),
    /// A group's constant alpha was NaN, infinite, or outside `0..=1`.
    InvalidGroupAlpha {
        /// The offending alpha.
        alpha: f32,
    },
    /// A [`ClipId`] that this scene never allocated. Clip identifiers are scene-scoped
    /// (§2.2 of the brief; `crate::ids`), so a foreign or stale one is a caller bug
    /// surfaced here rather than a wrong picture later.
    UnknownClip {
        /// The identifier that was presented.
        clip: ClipId,
        /// How many clips this scene has allocated; valid identifiers are below this.
        allocated: u32,
    },
    /// Group nesting exceeded [`MAX_GROUP_DEPTH`](crate::scene::MAX_GROUP_DEPTH).
    GroupTooDeep {
        /// The bound that was hit.
        limit: usize,
    },
    /// A group named a compositing operator it cannot be composited with (ADR 0033).
    GroupComposeUnsupported {
        /// What was asked for.
        compose: Compose,
        /// Which condition refused it.
        reason: GroupComposeReason,
    },
    /// One of §11.4.6's staged operators ([`Compose::DestOut`], [`Compose::Plus`]) in a
    /// position that already stages the clause by another route.
    StagedComposeUnsupported {
        /// The operator that was asked for.
        compose: Compose,
        /// Which position refused it.
        reason: StagedComposeReason,
    },
    /// A non-isolated group (§11.4.4) in a position where a one-accumulator raster
    /// cannot draw it. The reason names which of the three conditions failed; see
    /// [`GroupSpec::isolated`](crate::scene::GroupSpec::isolated) for why each is
    /// load-bearing, and ADR 0019 for the derivation.
    NonIsolatedGroupUnsupported {
        /// Which condition the scene broke.
        reason: NonIsolatedReason,
    },
    /// A [`MaskId`] that this scene has not (yet) defined. Masks are scene-scoped
    /// and may only be referenced after their `mask()` call returns — which also
    /// makes mask dependencies acyclic by construction.
    UnknownMask {
        /// The identifier that was presented.
        mask: MaskId,
        /// How many masks are defined so far; valid identifiers are below this.
        defined: u32,
    },
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteRect(rect) => {
                write!(f, "rectangle has a non-finite coordinate: {rect:?}")
            }
            Self::UnorderedRect(rect) => {
                write!(f, "rectangle min exceeds max: {rect:?}")
            }
            Self::RectTooLarge { rect, limit } => {
                write!(
                    f,
                    "rectangle coordinate exceeds the limit of {limit}: {rect:?}"
                )
            }
            Self::NonFiniteTransform(t) => {
                write!(f, "transform has a non-finite coefficient: {t:?}")
            }
            Self::TransformTooLarge { transform, limit } => {
                write!(
                    f,
                    "transform coefficient exceeds the limit of {limit}: {transform:?}"
                )
            }
            Self::InvalidColor(c) => {
                write!(f, "colour component non-finite or outside 0..=1: {c:?}")
            }
            Self::InvalidShading => {
                write!(
                    f,
                    "shading geometry non-finite, out of bounds, or negatively sized"
                )
            }
            Self::InvalidStroke(s) => {
                write!(
                    f,
                    "stroke width must be finite, positive and bounded, and the miter \
                     limit at least 1: {s:?}"
                )
            }
            Self::InvalidGroupAlpha { alpha } => {
                write!(f, "group alpha non-finite or outside 0..=1: {alpha}")
            }
            Self::UnknownClip { clip, allocated } => {
                write!(
                    f,
                    "clip {clip:?} was never allocated by this scene ({allocated} clips exist); \
                     clip identifiers are scene-scoped"
                )
            }
            Self::GroupTooDeep { limit } => {
                write!(f, "group nesting exceeds the bound of {limit}")
            }
            Self::GroupComposeUnsupported { compose, reason } => write!(
                f,
                "a group cannot be composited with {compose:?} here, because {}",
                reason.because()
            ),
            Self::StagedComposeUnsupported { compose, reason } => {
                let because = reason.because();
                write!(
                    f,
                    "{compose:?} states §11.4.6's second stage, and cannot be drawn here \
                     because {because}"
                )
            }
            Self::NonIsolatedGroupUnsupported { reason } => {
                let because = match reason {
                    NonIsolatedReason::GroupBlendNotNormal => {
                        "its own blend mode is not Normal, and the Normal composite is \
                         what cancels §11.4.4's backdrop removal"
                    }
                    NonIsolatedReason::KnockoutGroup => "it is also a knockout group",
                    NonIsolatedReason::InsideKnockoutGroup => {
                        "it is nested inside a knockout group"
                    }
                };
                write!(
                    f,
                    "a non-isolated group (§11.4.4) cannot be drawn here because {because}"
                )
            }
            Self::UnknownMask { mask, defined } => {
                write!(
                    f,
                    "mask {mask:?} is not defined by this scene ({defined} masks exist); masks \
                     are scene-scoped and referenced only after definition"
                )
            }
        }
    }
}

impl Error for SceneError {}

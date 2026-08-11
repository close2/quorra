//! The scene, and the builder that produces one.
//!
//! This is the centre of the library's design, and `doc/RENDER_LIBRARY.md` §2.3 states
//! the property that decides it:
//!
//! > The single most important property in this document: a `Scene` must contain no
//! > reference to a viewport, a resolution, a device transform, or a target size.
//!
//! Zoom, scroll, window resize and tiled output are all *the same scene at a different
//! viewport*. If building a scene were a function of the target, every zoom step would
//! redo it — and encoding measured 1.1–1.6 ms, flat across a sixteenfold range of
//! resolutions, which is 22% of a thumbnail's frame and 1.5 ms per frame the caller's
//! interpreter is not getting.
//!
//! The corollary, which §2.3 asks to have stated in our documentation rather than left
//! implicit: **a [`Scene`] is `Send + Sync`, cheap to clone, and building one requires
//! no device.** In this crate that is structural rather than aspirational — there is no
//! device type in scope to require. See `doc/adr/0001`.
//!
//! # Validation is here, and it is loud
//!
//! The builder is the boundary that structured input from another process's parser
//! crosses, so §4.7's rule is enforced here: non-finite coordinates, unordered
//! rectangles, out-of-range colours, invalid strokes, unknown clip identifiers,
//! groups past their depth bound and coordinates beyond [`MAX_COORDINATE`] are refused
//! with a typed [`SceneError`] naming what was wrong — never clamped, repaired, or
//! turned into NaN geometry downstream.
//!
//! # State: M6
//!
//! The vocabulary is the brief's §2.3 minus M7's images: `fill`, `stroke`, `rect`,
//! `clip`, `group` and `mask` all exist. One deliberate divergence from the brief's
//! illustrative signatures, recorded here and in `doc/PLAN.md` integration note 8:
//! the `mask` parameter comes **last** in each builder method rather than beside
//! `clip`, so that growing the vocabulary was a mechanical widening for every call
//! site. A scene may *hold* every command; what a device cannot yet *draw* (M7's
//! images) is refused loudly at render time, never approximated (§5).
//!
//! # What a `Scene` may not become
//!
//! Not a scene graph, not a retained widget tree, no animation and no timeline (§9). It
//! is built by an interpreter and thrown away when the page changes. §11.5 asks what
//! one costs to hold, against a target of a dozen resident pages out of a 1 023-page
//! document — [`Scene::cost`] is the running answer.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use crate::blend::{BlendMode, Compose, FillRule};
use crate::geom::{Affine, Rect};
use crate::ids::ClipId;
use crate::ids::{ImageId, MaskId, OutlineId};
use crate::mask::{MaskKind, Transfer};
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

/// How an image's samples map to pixels for one placement — the caller's
/// already-taken decision (§4.5), never re-taken here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFilter {
    /// Nearest sample: §8.9.5.3's default (`/Interpolate` false), and what three
    /// reference renderers draw for a magnified image.
    Nearest,
    /// Linear interpolation: `/Interpolate` true, or a placement the caller decided
    /// to smooth.
    Linear,
}

/// The deepest a group may nest. The brief's §1.1 bounds the caller's display list at
/// 16, so a deeper scene means something upstream went wrong, and the builder refuses
/// it rather than letting a device discover it mid-frame.
pub const MAX_GROUP_DEPTH: usize = 16;

/// What a transparency group is composited with once its content is complete
/// (ISO 32000-2 §11.4.4, §11.4.5): the group's elements draw onto the backdrop
/// [`GroupSpec::isolated`] names, and the finished group is painted exactly once under
/// these parameters.
///
/// [`GroupSpec::isolated`] is Table 145's `/I`, and it is the one entry whose default —
/// `true` — is the behaviour every rasterising library offers. See its documentation
/// for what `false` costs a backend and for the three conditions a scene must meet
/// before the builder will accept it (ADR 0019).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroupSpec {
    /// The group's constant alpha, `0..=1` (§11.4.5's group alpha).
    pub alpha: f32,
    /// How the finished group combines with its backdrop (§11.3.5).
    pub blend: BlendMode,
    /// Active clip for the composited group, or `None` for unclipped.
    pub clip: Option<ClipId>,
    /// Whether the group is a knockout group (§11.4.6): each element composites with
    /// the group's initial backdrop rather than with the stack of preceding elements.
    pub knockout: bool,
    /// Soft mask applied to the composited group, or `None` (§11.6.4.3).
    pub mask: Option<MaskId>,
    /// What the elements composite **onto** (ISO 32000-2 §11.4.5, §11.4.4).
    ///
    /// `true` is §11.4.5's isolated group, which is what a layer in any rasterising
    /// library is:
    ///
    /// > An isolated group is one whose elements shall be composited onto a fully
    /// > transparent initial backdrop rather than onto the group's backdrop.
    ///
    /// `false` is §11.4.4's own model: the elements composite onto the backdrop the
    /// group is being painted over, and the Result step then takes that backdrop's
    /// contribution out again so it is counted once (its NOTE 3 — "Essentially, these
    /// formulas remove the contribution of the group backdrop from the computed
    /// results"). The two differ only where an element **blends**, which §11.4.4's
    /// NOTE 2 gives as the whole reason both kinds exist.
    ///
    /// # The three conditions, and why the builder enforces them
    ///
    /// §11.4.4's removal divides by Table 140's *group alpha* — the elements' own
    /// accumulated alpha, "excluding the initial backdrop" — which is not the alpha a
    /// premultiplied raster holds; NOTE 4's advice is to keep a second set of
    /// accumulators for it. quorra keeps one set and does not need the second, because
    /// the quantity the removal divides out is multiplied straight back in when the
    /// group's result is composited with that same backdrop under the **Normal** blend
    /// function. Writing `B` for the backdrop, `E(B)` for the elements composited onto
    /// it — both premultiplied — and `w` for [`GroupSpec::alpha`] times the group's
    /// soft mask and clip at the pixel, the two steps together are
    ///
    /// ```text
    /// result = (1 − w) × B + w × E(B)
    /// ```
    ///
    /// Checked against transcriptions of §11.4.4's recurrence, its Result step and
    /// §11.3.6's composite over 200 000 random inputs: worst deviation 5.6 × 10⁻¹⁶
    /// (`quorra-gpu/tests/non_isolated_groups.rs`, which is that transcription).
    ///
    /// The step that cancels is the composite under **Normal**. Under any other blend
    /// the group's own colour is needed, and with it the group alpha this raster does
    /// not have — the same test measures 0.91 of full scale of error. So a
    /// non-isolated group is accepted only where [`GroupSpec::blend`] is
    /// [`BlendMode::Normal`], [`GroupSpec::knockout`] is `false`, and no enclosing
    /// group is a knockout group; anything else is a
    /// [`SceneError::NonIsolatedGroupUnsupported`], refused at the builder rather than
    /// approximated at the device (§5 of the brief).
    pub isolated: bool,
}

/// One soft mask: a transparency group plus the reduction that turns its pixels into
/// mask values (ISO 32000-2 §11.5; [`crate::mask`] carries the clause text).
///
/// A mask may only reference masks defined before it, which the builder enforces —
/// so mask dependencies are acyclic by construction, the same way clip chains are.
#[derive(Debug, Clone, PartialEq)]
pub struct MaskDef {
    /// Which of §11.5's two rules reduces the rendered group.
    pub kind: MaskKind,
    /// §11.6.5.1's `/TR` as a 256-entry table, or `None` for the identity.
    pub transfer: Option<Transfer>,
    /// The mask group's content, drawn onto transparency at device resolution.
    pub commands: Vec<Command>,
}

/// One clip region: an outline, a rule, and an optional parent, so that a chain is an
/// intersection (§4.7 of the brief). An **empty clip admits nothing**, which is a
/// different thing from an absent clip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipDef {
    /// The clipping outline, uploaded to the device the scene will render on.
    pub outline: OutlineId,
    /// The clip's absolute transform.
    pub transform: Affine,
    /// Which points the outline admits.
    pub rule: FillRule,
    /// The chain's next link: this clip intersected with its parent, recursively.
    pub parent: Option<ClipId>,
}

/// One drawing command, carrying its own absolute transform.
///
/// Nothing is inherited from a position in the list (§1.1 of the brief), which is what
/// lets a device reorder and parallelise without the result changing (§4.6). `Group` is
/// the one nested command, bounded at [`MAX_GROUP_DEPTH`].
///
/// The enum is exhaustive on purpose, and milestones extend it breakingly: a device
/// that forgets to handle a new command must fail to compile, not fall through a
/// wildcard arm. M6 adds the soft-mask reference to the drawing variants; M7 adds
/// `Image`.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Fill an axis-aligned rectangle with a solid colour, compositing with
    /// `BlendMode::Normal` over what is below.
    ///
    /// Not a special case of a path fill: §6.4. A rectangle is exact analytic coverage
    /// in a fragment shader — no tiling, no binning, no edge list — and it is what
    /// rules, backgrounds, underlines, table cells and *most clips* are. M3 extends it
    /// with clip state.
    Rect {
        /// The rectangle, in the scene's own coordinate space.
        rect: Rect,
        /// The command's absolute transform.
        transform: Affine,
        /// Solid device-RGB colour with straight alpha.
        color: Color,
        /// Active clip, or `None` for unclipped. An empty clip admits nothing.
        clip: Option<ClipId>,
        /// Active soft mask, or `None` (§11.5.1 calls a soft mask a *soft clip*,
        /// which is why it travels beside the clip).
        mask: Option<MaskId>,
    },
    /// Fill an uploaded outline (ISO 32000-2 §8.5.3.3's two rules, §11 for the
    /// compositing). Drawable once the glyph and path lanes exist (M4/M5).
    Fill {
        /// The outline, uploaded once and referenced per occurrence (§2.2).
        outline: OutlineId,
        /// The command's absolute transform.
        transform: Affine,
        /// Which points are inside (§8.5.3.3).
        rule: FillRule,
        /// How the interior is painted.
        paint: Paint,
        /// Active clip, or `None` for unclipped.
        clip: Option<ClipId>,
        /// How the result combines with the backdrop (§11.3.5).
        blend: BlendMode,
        /// The compositing behaviour — §4.1's coverage-modulated source is the second
        /// variant and the reason the field exists.
        compose: Compose,
        /// Active soft mask, or `None`.
        mask: Option<MaskId>,
    },
    /// Stroke an uploaded outline (§8.4.3). Drawable once the path lane exists (M5).
    Stroke {
        /// The outline to stroke.
        outline: OutlineId,
        /// The command's absolute transform.
        transform: Affine,
        /// Width, caps, joins — resolved upstream where §4.5 says so.
        stroke: Stroke,
        /// How the stroke is painted.
        paint: Paint,
        /// Active clip, or `None` for unclipped.
        clip: Option<ClipId>,
        /// How the result combines with the backdrop (§11.3.5).
        blend: BlendMode,
        /// Active soft mask, or `None`.
        mask: Option<MaskId>,
    },
    /// Draw a decoded image into the unit square (ISO 32000-2 §8.9.5: the image's
    /// top row sits at y = 1 of the unit square; the transform carries everything
    /// else, including PDF's y-up flip).
    Image {
        /// The uploaded image.
        image: ImageId,
        /// Maps the unit square into the scene's space.
        transform: Affine,
        /// Constant alpha applied on top of the image's own, `0..=1`.
        alpha: f32,
        /// The **resolved** filtering decision for this placement — §4.5's
        /// `/Interpolate` and the area-averaging departure are settled upstream,
        /// per placement, which is why this sits on the command and not on the
        /// uploaded resource (integration note 1).
        filter: ImageFilter,
        /// Active clip, or `None`.
        clip: Option<ClipId>,
        /// How the result combines with the backdrop (§11.3.5).
        blend: BlendMode,
        /// Active soft mask, or `None`.
        mask: Option<MaskId>,
    },
    /// A transparency group (§11.4): the nested commands draw onto transparency, and
    /// the finished group is painted once under its spec. Drawable at M6.
    Group {
        /// How the finished group is composited.
        spec: GroupSpec,
        /// The group's content, in order.
        commands: Vec<Command>,
    },
}

/// Which of [`GroupSpec::isolated`]'s three conditions a non-isolated group broke.
///
/// Each names a case where §11.4.4's backdrop removal no longer cancels against the
/// composite that follows it, so the group's own alpha — Table 140's group alpha,
/// which a premultiplied raster does not hold — would be needed to draw it correctly.
/// A refusal here is §5 of the brief: a hole and a sentence beat a plausible lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonIsolatedReason {
    /// The group's own blend mode is not [`BlendMode::Normal`]. The cancellation *is*
    /// the Normal blend function; under any other the identity is false by up to 0.91
    /// of full scale.
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

/// Why one of §11.4.6's staged operators cannot be drawn where it was placed.
///
/// [`Compose::DestOut`] and [`Compose::Plus`] are a caller's own expansion of §11.4.6's
/// second stage. Both positions below already *are* that stage by another route, so a
/// staged mark inside them would apply it twice — a refusal rather than a guess (§5 of
/// the brief).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagedComposeReason {
    /// The mark carries a blend mode other than [`BlendMode::Normal`], which wraps it in
    /// an implicit one-element group (§11.3.5) — so the operator would compose the group
    /// rather than the element, which is not where the clause puts it.
    BlendNotNormal,
    /// The mark is inside a knockout group, whose every element is already composited
    /// with the group's initial backdrop by the erase-and-deposit pair of §11.4.6.
    InsideKnockoutGroup,
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
    /// A rectangle coordinate exceeded [`MAX_COORDINATE`] in magnitude.
    RectTooLarge {
        /// The offending rectangle.
        rect: Rect,
        /// The limit it exceeded, named per §5 of the brief.
        limit: f32,
    },
    /// A transform had a NaN or infinite coefficient.
    NonFiniteTransform(Affine),
    /// A transform coefficient exceeded [`MAX_COORDINATE`] in magnitude.
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
    /// Group nesting exceeded [`MAX_GROUP_DEPTH`].
    GroupTooDeep {
        /// The bound that was hit.
        limit: usize,
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
    /// [`GroupSpec::isolated`] for why each is load-bearing, and ADR 0019 for the
    /// derivation.
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
            Self::StagedComposeUnsupported { compose, reason } => {
                let because = match reason {
                    StagedComposeReason::BlendNotNormal => {
                        "it also carries a blend mode, which puts it in an implicit \
                         one-element group (§11.3.5)"
                    }
                    StagedComposeReason::InsideKnockoutGroup => {
                        "it is inside a knockout group, which already stages §11.4.6 per \
                         element"
                    }
                };
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

/// What a scene costs, so that a limit can be discovered *before* a frame (§5's second
/// preference): a caller compares this against `Device::limits` and falls back rather
/// than discovering a refusal afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cost {
    /// Number of drawing commands, counted through group nesting.
    pub commands: usize,
    /// Number of clip regions the scene defines.
    pub clips: usize,
    /// Number of soft masks the scene defines.
    pub masks: usize,
    /// Deepest group nesting the scene reaches.
    pub group_depth: usize,
    /// Heap bytes the scene retains while held — §11.5's question, measured per scene.
    pub retained_bytes: usize,
}

/// What is to be drawn: an immutable, device-independent description of marks.
///
/// `Send + Sync`, cheap to clone (an `Arc` inside), and containing no reference to a
/// viewport, a resolution, a device transform or a target size — the brief's §2.3, held
/// structurally (`doc/adr/0001`). A blank scene is a legitimate scene and renders to a
/// legitimate, empty frame (§5).
#[derive(Debug, Clone)]
pub struct Scene {
    data: Arc<SceneData>,
}

#[derive(Debug)]
struct SceneData {
    commands: Vec<Command>,
    clips: Vec<ClipDef>,
    masks: Vec<MaskDef>,
    cost: Cost,
}

impl Scene {
    /// The top-level commands, in the order the builder received them.
    ///
    /// A device may draw them in any order whose result is identical (§4.6).
    #[must_use]
    pub fn commands(&self) -> &[Command] {
        &self.data.commands
    }

    /// The clip definitions, indexed by [`ClipId`]. A chain is resolved by following
    /// `parent` links; every link is valid by construction (the builder refused
    /// anything else).
    #[must_use]
    pub fn clips(&self) -> &[ClipDef] {
        &self.data.clips
    }

    /// The soft-mask definitions, indexed by [`MaskId`]. A mask's own commands may
    /// reference only masks defined before it, so realisation in index order is
    /// always possible.
    #[must_use]
    pub fn masks(&self) -> &[MaskDef] {
        &self.data.masks
    }

    /// What this scene costs, comparable against `Device::limits` before a frame is
    /// attempted. Computed once, at [`SceneBuilder::finish`]; asking is free.
    #[must_use]
    pub fn cost(&self) -> Cost {
        self.data.cost
    }
}

/// Builds a [`Scene`], validating every input at this boundary (§4.7).
///
/// Requires no device, runs on any thread — the caller's interpreter builds scenes on a
/// worker thread while the GPU is still initialising (§2.3). Resource identifiers
/// ([`OutlineId`] and friends) are opaque here: they belong to a device, and the device
/// validates them against its own registry at render time.
#[derive(Debug, Default)]
pub struct SceneBuilder {
    /// Finished top-level commands.
    commands: Vec<Command>,
    /// One frame per open group or mask body; commands land in the innermost frame.
    open_frames: Vec<OpenFrame>,
    clips: Vec<ClipDef>,
    masks: Vec<MaskDef>,
}

#[derive(Debug)]
struct OpenFrame {
    commands: Vec<Command>,
    /// Whether the commands landing here sit inside a knockout group — the question
    /// [`NonIsolatedReason::InsideKnockoutGroup`] asks. A soft mask's body starts a
    /// fresh stack: §11.6.5 renders the mask group on its own, so a knockout group
    /// outside it is not above the mask's content.
    inside_knockout: bool,
}

impl SceneBuilder {
    /// An empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an axis-aligned rectangle filled with a solid colour.
    ///
    /// An *empty* rectangle (zero width or height) is accepted and draws nothing, the
    /// same way a blank scene is legitimate.
    ///
    /// # Errors
    ///
    /// Refuses, without appending, any input §4.7 forbids: non-finite or unordered
    /// rectangles, non-finite transforms, coordinates or coefficients beyond
    /// [`MAX_COORDINATE`], colours outside their range, and clip identifiers this
    /// scene never allocated. The error names the value and the limit; nothing is
    /// clamped or repaired.
    pub fn rect(
        &mut self,
        rect: Rect,
        transform: Affine,
        color: Color,
        clip: Option<ClipId>,
        mask: Option<MaskId>,
    ) -> Result<(), SceneError> {
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
        Self::check_transform(transform)?;
        if !color.is_valid() {
            return Err(SceneError::InvalidColor(color));
        }
        self.check_clip(clip)?;
        self.check_mask(mask)?;
        self.push(Command::Rect {
            rect,
            transform,
            color,
            clip,
            mask,
        });
        Ok(())
    }

    /// Append a fill of an uploaded outline (§2.3 of the brief). The soft-mask
    /// parameter of the brief's signature arrives with M6, which owns masks entirely.
    ///
    /// # Errors
    ///
    /// Refuses invalid transforms, paints, and clip identifiers this scene never
    /// allocated. The outline identifier is opaque here; the device validates it
    /// against its registry at render time.
    // The brief's §2.3 signature, kept argument-for-argument so the caller's encoder
    // stays mechanical (one command, one call, no packing step). When M6 adds the
    // mask parameter this is revisited against a params struct, with the caller.
    #[allow(clippy::too_many_arguments)]
    pub fn fill(
        &mut self,
        outline: OutlineId,
        transform: Affine,
        rule: FillRule,
        paint: Paint,
        clip: Option<ClipId>,
        blend: BlendMode,
        compose: Compose,
        mask: Option<MaskId>,
    ) -> Result<(), SceneError> {
        Self::check_transform(transform)?;
        Self::check_paint(paint)?;
        self.check_clip(clip)?;
        self.check_mask(mask)?;
        self.check_staged_compose(compose, blend)?;
        self.push(Command::Fill {
            outline,
            transform,
            rule,
            paint,
            clip,
            blend,
            compose,
            mask,
        });
        Ok(())
    }

    /// Append a stroke of an uploaded outline (§2.3 of the brief; parameters resolved
    /// upstream per §4.5). The soft-mask parameter arrives with M6.
    ///
    /// # Errors
    ///
    /// Refuses invalid transforms, strokes ([`Stroke::is_valid`]), paints, and clip
    /// identifiers this scene never allocated.
    // The brief's §2.3 signature (see the `fill` note; integration note 8 records
    // the mask-last divergence).
    #[allow(clippy::too_many_arguments)]
    pub fn stroke(
        &mut self,
        outline: OutlineId,
        transform: Affine,
        stroke: Stroke,
        paint: Paint,
        clip: Option<ClipId>,
        blend: BlendMode,
        mask: Option<MaskId>,
    ) -> Result<(), SceneError> {
        Self::check_transform(transform)?;
        if !stroke.is_valid() {
            return Err(SceneError::InvalidStroke(stroke));
        }
        Self::check_paint(paint)?;
        self.check_clip(clip)?;
        self.check_mask(mask)?;
        self.push(Command::Stroke {
            outline,
            transform,
            stroke,
            paint,
            clip,
            blend,
            mask,
        });
        Ok(())
    }

    /// Allocate a clip region: an outline under a transform, admitting points by
    /// `rule`, intersected with `parent` when given (a chain is an intersection,
    /// §4.7). Returns the identifier commands reference it by — scene-scoped, meaning
    /// nothing to any other scene.
    ///
    /// # Errors
    ///
    /// Refuses invalid transforms and a `parent` this scene never allocated.
    pub fn clip(
        &mut self,
        outline: OutlineId,
        transform: Affine,
        rule: FillRule,
        parent: Option<ClipId>,
    ) -> Result<ClipId, SceneError> {
        Self::check_transform(transform)?;
        self.check_clip(parent)?;
        // The count fits u32 or the scene has bigger problems; the check keeps the
        // conversion honest rather than expecting it.
        let id = u32::try_from(self.clips.len()).map_err(|_| SceneError::UnknownClip {
            clip: ClipId(u32::MAX),
            allocated: u32::MAX,
        })?;
        self.clips.push(ClipDef {
            outline,
            transform,
            rule,
            parent,
        });
        Ok(ClipId(id))
    }

    /// Append a transparency group (ISO 32000-2 §11.4): everything `body` draws
    /// composites onto the backdrop [`GroupSpec::isolated`] names, and the finished
    /// group is painted exactly once under `spec`. Nesting is bounded at
    /// [`MAX_GROUP_DEPTH`].
    ///
    /// `body` returns a `Result` so builder refusals inside the group propagate with
    /// `?` — a small departure from the brief's illustrative closure, in exchange for
    /// no refusal ever being swallowed.
    ///
    /// # Errors
    ///
    /// Refuses an invalid alpha, an unknown clip, nesting beyond the bound, a
    /// non-isolated group in a position §11.4.4's arithmetic cannot survive
    /// ([`SceneError::NonIsolatedGroupUnsupported`]), and whatever `body` itself
    /// refuses. On any error the group is discarded whole; the builder remains usable
    /// and consistent.
    pub fn group(
        &mut self,
        spec: GroupSpec,
        body: impl FnOnce(&mut Self) -> Result<(), SceneError>,
    ) -> Result<(), SceneError> {
        if !spec.alpha.is_finite() || !(0.0..=1.0).contains(&spec.alpha) {
            return Err(SceneError::InvalidGroupAlpha { alpha: spec.alpha });
        }
        self.check_clip(spec.clip)?;
        self.check_mask(spec.mask)?;
        self.check_isolation(&spec)?;
        let commands = self.nested_body(self.inside_knockout() || spec.knockout, body)?;
        self.push(Command::Group { spec, commands });
        Ok(())
    }

    /// The three conditions of [`GroupSpec::isolated`], in the order a reader of the
    /// clause meets them. Checked before the body runs, so a refusal costs nothing
    /// that was built inside it.
    fn check_isolation(&self, spec: &GroupSpec) -> Result<(), SceneError> {
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

    /// Append an image draw (ISO 32000-2 §8.9.5): the uploaded image mapped into
    /// the unit square under `transform`, under a constant alpha and the placement's
    /// **resolved** filter (integration note 1). The image identifier is opaque
    /// here; the device validates it at render time.
    ///
    /// # Errors
    ///
    /// Refuses non-finite transforms, an alpha outside `0..=1`, and clip or mask
    /// identifiers this scene never allocated.
    #[allow(clippy::too_many_arguments)] // the brief's §2.3 signature, mask last (note 8)
    pub fn image(
        &mut self,
        image: ImageId,
        transform: Affine,
        alpha: f32,
        filter: ImageFilter,
        clip: Option<ClipId>,
        blend: BlendMode,
        mask: Option<MaskId>,
    ) -> Result<(), SceneError> {
        Self::check_transform(transform)?;
        if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
            return Err(SceneError::InvalidGroupAlpha { alpha });
        }
        self.check_clip(clip)?;
        self.check_mask(mask)?;
        self.push(Command::Image {
            image,
            transform,
            alpha,
            filter,
            clip,
            blend,
            mask,
        });
        Ok(())
    }

    /// Define a soft mask (ISO 32000-2 §11.5): `body` draws the mask's transparency
    /// group exactly as it would draw page content; the device renders it at device
    /// resolution and reduces it by `kind`, then `transfer` when given. Returns the
    /// identifier commands and groups reference it by — scene-scoped, and only valid
    /// *after* this call, which is what keeps mask dependencies acyclic.
    ///
    /// # Errors
    ///
    /// Refuses nesting beyond [`MAX_GROUP_DEPTH`] and whatever `body` itself
    /// refuses; on any error no mask is defined.
    pub fn mask(
        &mut self,
        kind: MaskKind,
        transfer: Option<Transfer>,
        body: impl FnOnce(&mut Self) -> Result<(), SceneError>,
    ) -> Result<MaskId, SceneError> {
        if let MaskKind::Luminosity { backdrop } = kind
            && !backdrop.is_valid()
        {
            return Err(SceneError::InvalidColor(backdrop));
        }
        // §11.6.5 renders the mask group on its own, so whatever encloses the `mask()`
        // call is not above the mask's content: the knockout stack starts fresh here.
        let commands = self.nested_body(false, body)?;
        let id = u32::try_from(self.masks.len()).unwrap_or(u32::MAX);
        self.masks.push(MaskDef {
            kind,
            transfer,
            commands,
        });
        Ok(MaskId(id))
    }

    /// Run a nested body against its own command frame, popping it on both paths so
    /// an errored body is discarded whole and the builder stays consistent.
    ///
    /// `inside_knockout` is what the *body's* commands are nested in, which is why the
    /// group's own knockout flag is folded in by the caller and a mask body passes
    /// `false`.
    fn nested_body(
        &mut self,
        inside_knockout: bool,
        body: impl FnOnce(&mut Self) -> Result<(), SceneError>,
    ) -> Result<Vec<Command>, SceneError> {
        if self.open_frames.len() >= MAX_GROUP_DEPTH {
            return Err(SceneError::GroupTooDeep {
                limit: MAX_GROUP_DEPTH,
            });
        }
        self.open_frames.push(OpenFrame {
            commands: Vec::new(),
            inside_knockout,
        });
        let body_result = body(self);
        let finished = self.open_frames.pop().unwrap_or(OpenFrame {
            commands: Vec::new(),
            inside_knockout,
        });
        body_result?;
        Ok(finished.commands)
    }

    /// Whether commands appended right now land inside a knockout group.
    fn inside_knockout(&self) -> bool {
        self.open_frames
            .last()
            .is_some_and(|frame| frame.inside_knockout)
    }

    /// Finish building. Consumes the builder; the scene is immutable from here on,
    /// and its [`Cost`] is computed here, once.
    #[must_use]
    pub fn finish(self) -> Scene {
        debug_assert!(
            self.open_frames.is_empty(),
            "group() and mask() close every frame they open, on both paths"
        );
        let cost = Self::measure(&self.commands, &self.clips, &self.masks);
        Scene {
            data: Arc::new(SceneData {
                commands: self.commands,
                clips: self.clips,
                masks: self.masks,
                cost,
            }),
        }
    }

    fn push(&mut self, command: Command) {
        match self.open_frames.last_mut() {
            Some(frame) => frame.commands.push(command),
            None => self.commands.push(command),
        }
    }

    fn check_mask(&self, mask: Option<MaskId>) -> Result<(), SceneError> {
        if let Some(mask) = mask {
            let defined = u32::try_from(self.masks.len()).unwrap_or(u32::MAX);
            if mask.0 >= defined {
                return Err(SceneError::UnknownMask { mask, defined });
            }
        }
        Ok(())
    }

    fn check_transform(transform: Affine) -> Result<(), SceneError> {
        if !transform.is_finite() {
            return Err(SceneError::NonFiniteTransform(transform));
        }
        let magnitude = transform
            .a
            .abs()
            .max(transform.b.abs())
            .max(transform.c.abs())
            .max(transform.d.abs())
            .max(transform.e.abs())
            .max(transform.f.abs());
        if magnitude > MAX_COORDINATE {
            return Err(SceneError::TransformTooLarge {
                transform,
                limit: MAX_COORDINATE,
            });
        }
        Ok(())
    }

    fn check_paint(paint: Paint) -> Result<(), SceneError> {
        if !paint.is_valid() {
            return Err(match paint {
                Paint::Solid(color) => SceneError::InvalidColor(color),
                Paint::Shading { .. } | Paint::Mesh(_) => SceneError::InvalidShading,
            });
        }
        Ok(())
    }

    /// §11.4.6's staged operators are the caller's own expansion of the clause, so the
    /// two positions that already expand it refuse them (ADR 0025).
    fn check_staged_compose(&self, compose: Compose, blend: BlendMode) -> Result<(), SceneError> {
        if matches!(compose, Compose::SrcOver | Compose::Src) {
            return Ok(());
        }
        let reason = if blend != BlendMode::Normal {
            Some(StagedComposeReason::BlendNotNormal)
        } else if self.inside_knockout() {
            Some(StagedComposeReason::InsideKnockoutGroup)
        } else {
            None
        };
        match reason {
            Some(reason) => Err(SceneError::StagedComposeUnsupported { compose, reason }),
            None => Ok(()),
        }
    }

    fn check_clip(&self, clip: Option<ClipId>) -> Result<(), SceneError> {
        if let Some(clip) = clip {
            let allocated = u32::try_from(self.clips.len()).unwrap_or(u32::MAX);
            if clip.0 >= allocated {
                return Err(SceneError::UnknownClip { clip, allocated });
            }
        }
        Ok(())
    }

    /// The cost walk: commands and depth through nesting (mask bodies included),
    /// plus a byte estimate of what the scene retains (§11.5).
    fn measure(commands: &[Command], clips: &[ClipDef], masks: &[MaskDef]) -> Cost {
        fn walk(commands: &[Command], depth: usize, cost: &mut Cost) {
            cost.group_depth = cost.group_depth.max(depth);
            for command in commands {
                cost.commands = cost.commands.saturating_add(1);
                cost.retained_bytes = cost.retained_bytes.saturating_add(size_of::<Command>());
                if let Command::Group { commands, .. } = command {
                    walk(commands, depth.saturating_add(1), cost);
                }
            }
        }
        let mut cost = Cost {
            clips: clips.len(),
            masks: masks.len(),
            retained_bytes: clips
                .len()
                .saturating_mul(size_of::<ClipDef>())
                .saturating_add(masks.len().saturating_mul(size_of::<MaskDef>()))
                .saturating_add(size_of::<SceneData>()),
            ..Cost::default()
        };
        walk(commands, 0, &mut cost);
        for mask in masks {
            walk(&mask.commands, 1, &mut cost);
        }
        cost
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Command, GroupSpec, MAX_COORDINATE, MAX_GROUP_DEPTH, Scene, SceneBuilder, SceneError,
    };
    use crate::blend::{BlendMode, Compose, FillRule};
    use crate::geom::{Affine, Point, Rect};
    use crate::ids::{ClipId, OutlineId};
    use crate::paint::{Color, LineCap, LineJoin, Paint, Stroke};

    fn unit_rect() -> Rect {
        Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0))
    }

    fn black() -> Color {
        Color::new(0.0, 0.0, 0.0, 1.0)
    }

    fn plain_group() -> GroupSpec {
        GroupSpec {
            alpha: 1.0,
            blend: BlendMode::Normal,
            clip: None,
            knockout: false,
            mask: None,
            isolated: true,
        }
    }

    /// §2.3's corollary, checked by the compiler: the day someone adds an `Rc` this
    /// stops compiling, rather than the caller's worker thread failing.
    #[test]
    fn scene_is_send_sync_and_static() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<Scene>();
    }

    /// A blank scene is a legitimate scene (§5): zero commands, zero cost, no error.
    #[test]
    fn blank_scene_is_legitimate() {
        let scene = SceneBuilder::new().finish();
        assert!(scene.commands().is_empty());
        assert_eq!(scene.cost().commands, 0);
    }

    /// Clones share the same data rather than copying it — "cheap to clone (an `Arc`
    /// inside)" is the brief's own phrasing and this pins it.
    #[test]
    fn clones_share_storage() {
        let mut builder = SceneBuilder::new();
        builder
            .rect(unit_rect(), Affine::IDENTITY, black(), None, None)
            .expect("valid input");
        let scene = builder.finish();
        let clone = scene.clone();
        assert_eq!(
            scene.commands().as_ptr(),
            clone.commands().as_ptr(),
            "a clone must reference the same command storage"
        );
    }

    /// An empty rectangle is accepted — it draws nothing, and drawing nothing is a
    /// legitimate thing for a scene to ask.
    #[test]
    fn empty_rect_is_accepted() {
        let mut builder = SceneBuilder::new();
        let empty = Rect::new(Point::new(3.0, 4.0), Point::new(3.0, 9.0));
        builder
            .rect(
                empty,
                Affine::IDENTITY,
                Color::new(1.0, 0.0, 0.0, 1.0),
                None,
                None,
            )
            .expect("an empty rect is legitimate");
        assert_eq!(builder.finish().commands().len(), 1);
    }

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

    /// A chain is an intersection: parents link backwards, and the definitions come
    /// back exactly as allocated.
    #[test]
    fn clip_chains_link_as_allocated() {
        let mut builder = SceneBuilder::new();
        let root = builder
            .clip(OutlineId(3), Affine::IDENTITY, FillRule::NonZero, None)
            .expect("valid clip");
        let child = builder
            .clip(
                OutlineId(4),
                Affine::scale(2.0, 2.0),
                FillRule::EvenOdd,
                Some(root),
            )
            .expect("valid child clip");
        builder
            .fill(
                OutlineId(5),
                Affine::IDENTITY,
                FillRule::NonZero,
                Paint::Solid(black()),
                Some(child),
                BlendMode::Multiply,
                Compose::SrcOver,
                None,
            )
            .expect("valid fill under a chain");
        let scene = builder.finish();
        assert_eq!(scene.clips().len(), 2);
        assert_eq!(
            scene.clips()[usize::try_from(child.0).unwrap()].parent,
            Some(root)
        );
        assert_eq!(scene.cost().clips, 2);
    }

    /// Nesting to the bound succeeds; one deeper is refused with the bound named, and
    /// the builder stays usable afterwards.
    #[test]
    fn group_depth_is_bounded_at_sixteen() {
        fn nest(builder: &mut SceneBuilder, remaining: usize) -> Result<(), SceneError> {
            if remaining == 0 {
                return builder.rect(
                    Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
                    Affine::IDENTITY,
                    Color::new(0.0, 0.0, 0.0, 1.0),
                    None,
                    None,
                );
            }
            builder.group(
                GroupSpec {
                    alpha: 1.0,
                    blend: BlendMode::Normal,
                    clip: None,
                    knockout: false,
                    mask: None,
                    isolated: true,
                },
                |b| nest(b, remaining - 1),
            )
        }

        let mut builder = SceneBuilder::new();
        nest(&mut builder, MAX_GROUP_DEPTH).expect("the bound itself is allowed");

        let mut too_deep = SceneBuilder::new();
        assert!(matches!(
            nest(&mut too_deep, MAX_GROUP_DEPTH + 1),
            Err(SceneError::GroupTooDeep {
                limit: MAX_GROUP_DEPTH
            })
        ));
        too_deep
            .rect(unit_rect(), Affine::IDENTITY, black(), None, None)
            .expect("the builder survives a refused group");
        let scene = builder.finish();
        assert_eq!(scene.cost().group_depth, MAX_GROUP_DEPTH);
        assert_eq!(scene.cost().commands, MAX_GROUP_DEPTH + 1);
    }

    /// An error inside a group discards the group whole — no half-built group is ever
    /// a scene's content — and propagates to the caller.
    #[test]
    fn errored_groups_are_discarded_whole() {
        let mut builder = SceneBuilder::new();
        let result = builder.group(plain_group(), |b| {
            b.rect(unit_rect(), Affine::IDENTITY, black(), None, None)?;
            b.rect(
                unit_rect(),
                Affine::IDENTITY,
                Color::new(2.0, 0.0, 0.0, 1.0),
                None,
                None,
            )
        });
        assert!(matches!(result, Err(SceneError::InvalidColor(_))));
        assert!(
            builder.finish().commands().is_empty(),
            "a group that errored must not appear in the scene"
        );
    }

    /// Commands land in their group, groups nest, and the whole shape comes back.
    #[test]
    fn groups_nest_and_round_trip() {
        let mut builder = SceneBuilder::new();
        builder
            .group(
                GroupSpec {
                    alpha: 0.5,
                    ..plain_group()
                },
                |b| {
                    b.rect(unit_rect(), Affine::IDENTITY, black(), None, None)?;
                    b.group(plain_group(), |inner| {
                        inner.rect(unit_rect(), Affine::IDENTITY, black(), None, None)
                    })
                },
            )
            .expect("valid nested groups");
        let scene = builder.finish();
        assert_eq!(scene.commands().len(), 1);
        let Command::Group { spec, commands } = &scene.commands()[0] else {
            panic!("expected a group at the top level");
        };
        assert!((spec.alpha - 0.5).abs() < f32::EPSILON);
        assert_eq!(commands.len(), 2);
        assert!(matches!(commands[1], Command::Group { .. }));
        // A group is itself a command: outer group + rect + inner group + inner rect.
        assert_eq!(scene.cost().commands, 4);
        assert_eq!(scene.cost().group_depth, 2);
        assert!(scene.cost().retained_bytes > 0);
    }
}

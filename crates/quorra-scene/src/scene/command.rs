//! The vocabulary a scene is written in: one command per mark, and the four
//! definitions a command may point at.
//!
//! Nothing here validates and nothing here draws — these types are what a
//! [`Scene`](super::Scene) *is*, which is why they carry the clause citations rather
//! than the builder that checks them or the device that executes them. ISO 32000-2
//! §11.4 for a group, §11.5 for a soft mask, §8.5.4 for a clip chain.
//!
//! The enum is exhaustive on purpose (see [`Command`]): a device that forgets a new
//! command fails to compile. That promise is a property of this module's shape, so it
//! is stated where the shape is.

use crate::blend::{BlendMode, Compose, FillRule};
use crate::geom::{Affine, Rect};
use crate::ids::ClipId;
use crate::ids::{ImageId, MaskId, OutlineId};
use crate::mask::{MaskKind, Transfer};
use crate::paint::{Color, Paint, Stroke};

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
    /// The group's constant alpha, `0..=1`.
    ///
    /// §11.6.4.4's nonstroking alpha constant, which "shall also be applied when
    /// painting a transparency group's results onto its backdrop", in the range
    /// §11.3.7.2 states for every shape and opacity input: "values in the range 0.0 to
    /// 1.0 (inclusive), with a default value of 1.0".
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
    /// How the finished group combines with its backdrop, for the one case where
    /// [`BlendMode`] cannot say it: §11.4.6's two stages (ADR 0033).
    ///
    /// [`Compose::SrcOver`] is the ordinary group and what every other entry of this
    /// struct assumes. [`Compose::DestOut`] and [`Compose::Plus`] write the clause's
    /// second stage — `P' = (1 − f) × P + S` — with a *group* as the source of each
    /// half, which is what §11.6.4.2 forces for a knockout element that is itself a
    /// group:
    ///
    /// > The shape of a group object shall be the union […] of the shapes of the objects
    /// > it contains.
    ///
    /// A caller states the erase half as the same group drawn opaque, so its alpha *is*
    /// its shape, and the deposit half as the group itself. [`Compose::Src`] is refused
    /// here: an element whose shape is its coverage is what [`GroupSpec::knockout`]
    /// already means.
    pub compose: Compose,
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
    /// [`SceneError::NonIsolatedGroupUnsupported`](crate::error::SceneError::NonIsolatedGroupUnsupported),
    /// refused at the builder rather than approximated at the device (§5 of the brief).
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

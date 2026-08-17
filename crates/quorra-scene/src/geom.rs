//! Points, rectangles, affine transforms and outline segments.
//!
//! # The contract
//!
//! - **`f32`, matching the caller.** Its `pdf_render::geom` is `f32` throughout, and a
//!   boundary that widened to `f64` and back would round twice for no gain. Where a
//!   computation genuinely needs more — a determinant, an accumulated bound — the wider
//!   type is used inside and named in a comment.
//! - **Move, line, cubic, close. No quadratics.** PDF has no quadratic operator, and
//!   TrueType outlines are elevated to cubics during glyph loading upstream, so the whole
//!   pipeline handles one curve type. A quadratic reaching us would mean somebody added a
//!   second curve type to the caller, which is a conversation, not a conversion.
//! - **A transform per command, absolute.** Nothing is inherited from a position in a
//!   list: this is what lets a device reorder and parallelise (§1.1 of the brief), and
//!   §4.6 forbids the result changing when it does.
//! - **The page's own space is y-up.** The y flip is in the viewport transform, not in
//!   the scene, and not here.
//! - **Very large coordinates and degenerate transforms arrive from real files.** §4.7:
//!   refuse them loudly; never produce NaN geometry. The refusal itself lives in
//!   [`crate::scene::SceneBuilder`], which is the boundary structured input crosses;
//!   these types make the check expressible — [`Affine::invert`] returns `None` rather
//!   than a silently-identity fallback, and [`Rect::is_finite`] exists to be called.
//!
//! # What is deliberately absent
//!
//! No path *type*. An outline reaches a device as `&[Segment]` and comes back as an
//! [`OutlineId`]; a `Path` struct here would be a second owner of geometry that the
//! caller already owns behind an `Arc`, and §2.2 wants upload separated from scene
//! building precisely so that the geometry lives in one place.
//!
//! # The three parts, and what each one's one thing is
//!
//! The parts are private modules re-exported from here (`doc/adr/0051`), so
//! `quorra_scene::geom::Affine` and `quorra_scene::Affine` both resolve exactly as they
//! did and **no new public path exists**. rustdoc inlines a re-export from a private
//! module, so this list is the only place the division survives into the documentation:
//!
//! - **`shape`** — where a mark is and how big: [`Point`], [`Size`], [`Rect`]. Every
//!   bound in this library is one of these, and none of them is arithmetic anybody has
//!   to reason about.
//! - **`affine`** — ISO 32000-2 §8.3.3's matrix, and the four questions other subsystems
//!   ask of one: is it inside §4.7's coordinate bound, does it preserve axes (§6.4), how
//!   far does it stretch (§6.3's atlas bucket), and does it invert.
//! - **`segment`** — [`Segment`], the one step an outline is made of, and
//!   [`axis_aligned_rect`], the one shape a run of them is *recognised* as. That
//!   recogniser is a decision about lanes rather than a property of a curve, which is why
//!   it sits with the type it reads rather than with the [`Rect`] it returns.
//!
//! [`OutlineId`]: crate::ids::OutlineId

mod affine;
mod segment;
mod shape;

pub use affine::Affine;
pub use segment::{Segment, axis_aligned_rect};
pub use shape::{Point, Rect, Size};

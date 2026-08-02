//! Points, rectangles, affine transforms and outline segments.
//!
//! **Skeleton — M2 fills this** (`doc/adr/0003`).
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
//!   list: this is what lets us reorder and parallelise (§1.1), and §4.6 forbids the
//!   result changing when we do.
//! - **The page's own space is y-up.** The y flip is in the viewport transform, not in
//!   the scene, and not here.
//! - **Very large coordinates and degenerate transforms arrive from real files.** §4.7:
//!   refuse them loudly; never produce NaN geometry. That check belongs to whatever
//!   consumes these types, but the types must make the check expressible — which is why
//!   there is no `Transform::invert` returning a silently-identity fallback.
//!
//! # Planned signatures
//!
//! ```text
//! pub struct Point { pub x: f32, pub y: f32 }
//! pub struct Size  { pub width: f32, pub height: f32 }
//! pub struct Rect  { pub min: Point, pub max: Point }
//!
//! /// The six numbers of a PDF matrix, in the clause's own order (§8.3.3).
//! pub struct Affine { pub a: f32, pub b: f32, pub c: f32, pub d: f32, pub e: f32, pub f: f32 }
//!
//! impl Affine {
//!     pub const IDENTITY: Affine;
//!     pub fn then(self, other: Affine) -> Affine;
//!     pub fn apply(self, p: Point) -> Point;
//!     pub fn determinant(self) -> f32;
//!     pub fn preserves_axes(self) -> bool;   // the rectangle fast path of §6.4 asks this
//!     pub fn max_stretch(self) -> f32;       // the atlas scale bucket of §6.3 asks this
//!     pub fn invert(self) -> Option<Affine>;
//! }
//!
//! pub enum Segment {
//!     MoveTo(Point),
//!     LineTo(Point),
//!     CubicTo { c1: Point, c2: Point, to: Point },
//!     Close,
//! }
//! ```
//!
//! # What is deliberately absent
//!
//! No path *type*. An outline reaches a device as `&[Segment]` and comes back as an
//! [`OutlineId`]; a `Path` struct here would be a second owner of geometry that the
//! caller already owns behind an `Arc`, and §2.2 wants upload separated from scene
//! building precisely so that the geometry lives in one place.
//!
//! [`OutlineId`]: crate::ids::OutlineId

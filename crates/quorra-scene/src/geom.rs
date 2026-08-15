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
//! [`OutlineId`]: crate::ids::OutlineId

/// A point in the scene's own coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: f32,
    /// Vertical coordinate, y-up in the page's own space (§3 of the brief).
    pub y: f32,
}

impl Point {
    /// A point from its two coordinates.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Whether both coordinates are finite — neither NaN nor infinite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// A width and a height, without a position.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    /// Horizontal extent.
    pub width: f32,
    /// Vertical extent.
    pub height: f32,
}

/// An axis-aligned rectangle: `min` is the corner with the smaller coordinates on both
/// axes, `max` the larger.
///
/// A rectangle with `min == max` on either axis is *empty*: it is a legitimate value
/// that covers no area and draws nothing. A rectangle with `min > max` on either axis is
/// not a value this library accepts anywhere, and [`Rect::is_ordered`] is the check.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// The corner with the smaller x and the smaller y.
    pub min: Point,
    /// The corner with the larger x and the larger y.
    pub max: Point,
}

impl Rect {
    /// A rectangle from its two extreme corners, as given.
    ///
    /// The corners are stored as passed; whether they are ordered is a property the
    /// consumer checks with [`Rect::is_ordered`], because a constructor that silently
    /// swapped corners would repair data that §4.7 says must be refused loudly.
    #[must_use]
    pub const fn new(min: Point, max: Point) -> Self {
        Self { min, max }
    }

    /// The rectangle's extent.
    #[must_use]
    pub fn size(self) -> Size {
        Size {
            width: self.max.x - self.min.x,
            height: self.max.y - self.min.y,
        }
    }

    /// Whether all four coordinates are finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.min.is_finite() && self.max.is_finite()
    }

    /// Whether `min <= max` on both axes. An unordered rectangle is refused at the
    /// scene boundary, never repaired.
    #[must_use]
    pub fn is_ordered(self) -> bool {
        self.min.x <= self.max.x && self.min.y <= self.max.y
    }

    /// Whether the rectangle covers no area — zero width or zero height (or both).
    ///
    /// An empty rectangle is a legitimate scene item that draws nothing, in the same
    /// way that a blank scene is a legitimate scene (§5 of the brief).
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.min.x >= self.max.x || self.min.y >= self.max.y
    }

    /// The intersection of two rectangles.
    ///
    /// A clip chain is an intersection (§4.7 of the brief), and for axis-aligned
    /// rectangles the intersection is again a rectangle: the greater of the minima,
    /// the lesser of the maxima. When the inputs do not overlap the result is empty
    /// per [`Rect::is_empty`] — which is exactly the "empty clip admits nothing"
    /// value, distinct from no clip at all.
    #[must_use]
    pub fn intersection(self, other: Self) -> Self {
        Self {
            min: Point::new(self.min.x.max(other.min.x), self.min.y.max(other.min.y)),
            max: Point::new(self.max.x.min(other.max.x), self.max.y.min(other.max.y)),
        }
    }
}

/// The axis-aligned rectangle an outline traces, if it traces exactly one.
///
/// §6.4 of the brief: most clips are rectangles, and a rectangular clip must become
/// four floats, never a mask texture. The caller's clips arrive as outlines (its
/// display list has no rectangle type), so the rectangle has to be *recognised*, and
/// this is the recogniser: one subpath of four axis-aligned line edges, closed
/// explicitly (`Close`), by returning to the start, or both. Anything else — curves,
/// more corners, oblique edges — is `None`, and its clip is the path lane's work (M5).
///
/// A *degenerate* rectangle (zero width or height) is recognised and returned: as a
/// clip it is the empty clip, which admits nothing and must not be confused with an
/// unrecognised one.
#[must_use]
pub fn axis_aligned_rect(segments: &[Segment]) -> Option<Rect> {
    // Accept M,L,L,L[,L-back-to-start][,Close] — nothing more, nothing less.
    let mut points: Vec<Point> = Vec::with_capacity(5);
    let mut iter = segments.iter();
    match iter.next() {
        Some(Segment::MoveTo(p)) => points.push(*p),
        _ => return None,
    }
    for segment in iter.by_ref() {
        match segment {
            Segment::LineTo(p) => {
                if points.len() > 5 {
                    return None;
                }
                points.push(*p);
            }
            Segment::Close => {
                if iter.next().is_some() {
                    return None;
                }
                break;
            }
            Segment::MoveTo(_) | Segment::CubicTo { .. } => return None,
        }
    }
    // Exact comparison on purpose throughout this function: a rectangle from a PDF
    // `re` operator carries exact coordinates, and a nearly-closed path is not a
    // rectangle.
    #[allow(clippy::float_cmp)]
    if points.len() == 5 && points[4].x == points[0].x && points[4].y == points[0].y {
        points.truncate(4);
    }
    let [p0, p1, p2, p3] = *points.as_slice() else {
        return None;
    };
    // Four edges (with wraparound), alternating vertical/horizontal in either phase.
    // Written with exact equality: see above.
    #[allow(clippy::float_cmp)]
    let is_rect = (p0.x == p1.x && p1.y == p2.y && p2.x == p3.x && p3.y == p0.y)
        || (p0.y == p1.y && p1.x == p2.x && p2.y == p3.y && p3.x == p0.x);
    if !is_rect {
        return None;
    }
    let min = Point::new(
        p0.x.min(p1.x).min(p2.x).min(p3.x),
        p0.y.min(p1.y).min(p2.y).min(p3.y),
    );
    let max = Point::new(
        p0.x.max(p1.x).max(p2.x).max(p3.x),
        p0.y.max(p1.y).max(p2.y).max(p3.y),
    );
    Some(Rect::new(min, max))
}

/// The six numbers of a PDF transformation matrix, in the clause's own order.
///
/// ISO 32000-2 §8.3.3 writes the matrix `[a b c d e f]` and maps a point by
/// x′ = a·x + c·y + e and y′ = b·x + d·y + f — the row-vector convention: the linear
/// part is `[[a b], [c d]]` applied to a row vector, `[e f]` the translation.
///
/// [`Affine::then`] composes in application order — `first.then(second)` maps a point
/// through `first`, then `second` — because a reader of `command.then(viewport)` should
/// not have to remember which side of a `*` means what.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    /// §8.3.3's `a`: the x-from-x coefficient.
    pub a: f32,
    /// §8.3.3's `b`: the y-from-x coefficient.
    pub b: f32,
    /// §8.3.3's `c`: the x-from-y coefficient.
    pub c: f32,
    /// §8.3.3's `d`: the y-from-y coefficient.
    pub d: f32,
    /// §8.3.3's `e`: the x translation.
    pub e: f32,
    /// §8.3.3's `f`: the y translation.
    pub f: f32,
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    /// The transform that maps every point to itself.
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// A pure translation.
    #[must_use]
    pub const fn translate(x: f32, y: f32) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: x,
            f: y,
        }
    }

    /// A pure scale about the origin.
    #[must_use]
    pub const fn scale(x: f32, y: f32) -> Self {
        Self {
            a: x,
            b: 0.0,
            c: 0.0,
            d: y,
            e: 0.0,
            f: 0.0,
        }
    }

    /// The transform that applies `self` first and `other` second.
    #[must_use]
    pub fn then(self, other: Self) -> Self {
        Self {
            a: self.a * other.a + self.b * other.c,
            b: self.a * other.b + self.b * other.d,
            c: self.c * other.a + self.d * other.c,
            d: self.c * other.b + self.d * other.d,
            e: self.e * other.a + self.f * other.c + other.e,
            f: self.e * other.b + self.f * other.d + other.f,
        }
    }

    /// The point `self` maps `p` to, per §8.3.3's equations quoted on the type.
    #[must_use]
    pub fn apply(self, p: Point) -> Point {
        Point {
            x: self.a * p.x + self.c * p.y + self.e,
            y: self.b * p.x + self.d * p.y + self.f,
        }
    }

    /// The determinant of the linear part, `a·d − b·c`.
    #[must_use]
    pub fn determinant(self) -> f32 {
        self.a * self.d - self.b * self.c
    }

    /// The largest magnitude among the six coefficients.
    ///
    /// The number §4.7's coordinate bound is applied to: a transform is refused when this
    /// exceeds [`MAX_COORDINATE`](crate::scene::MAX_COORDINATE), and it lives here so
    /// that every boundary asking that question asks it the same way.
    #[must_use]
    pub fn max_coefficient(self) -> f32 {
        self.a
            .abs()
            .max(self.b.abs())
            .max(self.c.abs())
            .max(self.d.abs())
            .max(self.e.abs())
            .max(self.f.abs())
    }

    /// Whether all six coefficients are finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.a.is_finite()
            && self.b.is_finite()
            && self.c.is_finite()
            && self.d.is_finite()
            && self.e.is_finite()
            && self.f.is_finite()
    }

    /// Whether this transform maps axis-aligned rectangles to axis-aligned rectangles.
    ///
    /// True for scales, translations, and quarter-turn rotations with or without flips
    /// — that is, when the linear part is diagonal (`b = c = 0`) or anti-diagonal
    /// (`a = d = 0`). The test is exact, deliberately: transforms in documents carry
    /// exact zeros, and a transform that is only *nearly* axis-preserving maps a
    /// rectangle to something that is not one, which is the general path's job to draw
    /// (§6.4 of the brief is about the rectangles that really are rectangles).
    #[must_use]
    pub fn preserves_axes(self) -> bool {
        // Exact comparison is the semantics, not an oversight: see the doc comment.
        #[allow(clippy::float_cmp)]
        {
            (self.b == 0.0 && self.c == 0.0) || (self.a == 0.0 && self.d == 0.0)
        }
    }

    /// The largest factor by which this transform stretches any direction — the largest
    /// singular value of the linear part.
    ///
    /// The atlas's scale bucket (§6.3 of the brief) asks this question: it is the number
    /// that says how big a glyph's device-space image is, whatever the rotation.
    #[must_use]
    pub fn max_stretch(self) -> f32 {
        // Largest eigenvalue of LᵀL for the linear part L, via the closed form for a
        // symmetric 2×2 matrix.
        let x = self.a * self.a + self.b * self.b;
        let y = self.c * self.c + self.d * self.d;
        let z = self.a * self.c + self.b * self.d;
        let half_diff = (x - y) * 0.5;
        let s_squared = (x + y) * 0.5 + (half_diff * half_diff + z * z).sqrt();
        s_squared.sqrt()
    }

    /// The inverse transform, or `None` when there is none to have.
    ///
    /// `None` for a degenerate (zero-determinant) or non-finite transform. There is no
    /// identity fallback on purpose: a silently-substituted identity is exactly the
    /// plausible-looking wrong answer §4.7 forbids.
    #[must_use]
    pub fn invert(self) -> Option<Self> {
        let det = self.determinant();
        // Exact zero test: division below is well-defined for every other value, and a
        // near-zero determinant inverts to legitimately huge (finite) coefficients.
        #[allow(clippy::float_cmp)]
        if !self.is_finite() || !det.is_finite() || det == 0.0 {
            return None;
        }
        let inv_det = 1.0 / det;
        Some(Self {
            a: self.d * inv_det,
            b: -self.b * inv_det,
            c: -self.c * inv_det,
            d: self.a * inv_det,
            e: (self.c * self.f - self.d * self.e) * inv_det,
            f: (self.b * self.e - self.a * self.f) * inv_det,
        })
    }
}

/// One step of an outline: move, line, cubic, close — and deliberately nothing else.
///
/// No quadratic variant exists because PDF has no quadratic operator and TrueType
/// outlines are elevated to cubics during glyph loading upstream, so one curve type
/// reaches this library (§1.1 of the brief).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    /// Begin a new subpath at the point.
    MoveTo(Point),
    /// A straight line from the current point.
    LineTo(Point),
    /// A cubic Bézier from the current point, with two control points.
    CubicTo {
        /// The first control point.
        c1: Point,
        /// The second control point.
        c2: Point,
        /// The end point.
        to: Point,
    },
    /// Close the current subpath back to its most recent `MoveTo`.
    Close,
}

#[cfg(test)]
mod tests {
    use super::{Affine, Point, Rect, Segment, axis_aligned_rect};

    /// `then` is application order: translate-then-scale multiplies the translation,
    /// scale-then-translate does not. Derivable by hand from §8.3.3's equations.
    #[test]
    fn composition_is_application_order() {
        let translate = Affine::translate(1.0, 2.0);
        let scale = Affine::scale(10.0, 100.0);

        let p = Point::new(0.0, 0.0);
        assert_eq!(translate.then(scale).apply(p), Point::new(10.0, 200.0));
        assert_eq!(scale.then(translate).apply(p), Point::new(1.0, 2.0));
    }

    /// Composing transforms and applying them one after the other are the same map.
    #[test]
    fn composition_agrees_with_sequential_application() {
        let first = Affine {
            a: 2.0,
            b: 0.5,
            c: -1.0,
            d: 3.0,
            e: 4.0,
            f: -2.0,
        };
        let second = Affine {
            a: 0.25,
            b: -1.5,
            c: 2.0,
            d: 1.0,
            e: -3.0,
            f: 0.5,
        };
        let p = Point::new(3.0, -7.0);
        let composed = first.then(second).apply(p);
        let sequential = second.apply(first.apply(p));
        assert!((composed.x - sequential.x).abs() < 1e-4);
        assert!((composed.y - sequential.y).abs() < 1e-4);
    }

    /// Diagonal and anti-diagonal linear parts preserve axis alignment; anything with a
    /// shear or a non-quarter rotation does not.
    #[test]
    fn preserves_axes_covers_quarter_turns_and_rejects_shears() {
        assert!(Affine::IDENTITY.preserves_axes());
        assert!(Affine::scale(2.0, -3.0).preserves_axes());
        // A quarter turn: x' = -y, y' = x.
        let quarter = Affine {
            a: 0.0,
            b: 1.0,
            c: -1.0,
            d: 0.0,
            e: 0.0,
            f: 0.0,
        };
        assert!(quarter.preserves_axes());
        let shear = Affine {
            a: 1.0,
            b: 0.0,
            c: 0.5,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        };
        assert!(!shear.preserves_axes());
    }

    /// The largest singular value of a pure scale is the larger scale factor; of a
    /// rotation, exactly one.
    #[test]
    fn max_stretch_of_scale_and_rotation() {
        assert!((Affine::scale(2.0, 3.0).max_stretch() - 3.0).abs() < 1e-6);
        let angle = 0.7_f32;
        let rotation = Affine {
            a: angle.cos(),
            b: angle.sin(),
            c: -angle.sin(),
            d: angle.cos(),
            e: 0.0,
            f: 0.0,
        };
        assert!((rotation.max_stretch() - 1.0).abs() < 1e-6);
    }

    /// A transform followed by its inverse is the identity, to rounding.
    #[test]
    fn invert_round_trips() {
        let t = Affine {
            a: 2.0,
            b: 1.0,
            c: -0.5,
            d: 3.0,
            e: 10.0,
            f: -4.0,
        };
        let inverse = t.invert().expect("determinant is nonzero");
        let p = Point::new(5.5, -2.25);
        let round_tripped = inverse.apply(t.apply(p));
        assert!((round_tripped.x - p.x).abs() < 1e-4);
        assert!((round_tripped.y - p.y).abs() < 1e-4);
    }

    /// Degenerate and non-finite transforms have no inverse — and no identity fallback.
    #[test]
    fn invert_refuses_degenerate_and_non_finite() {
        assert!(Affine::scale(0.0, 1.0).invert().is_none());
        let non_finite = Affine {
            a: f32::NAN,
            ..Affine::IDENTITY
        };
        assert!(non_finite.invert().is_none());
    }

    /// `is_ordered` and `is_empty` are distinct questions: an empty rectangle is
    /// ordered and legitimate; an unordered one is refused input.
    #[test]
    fn rect_ordered_and_empty_are_distinct() {
        let empty = Rect::new(Point::new(1.0, 1.0), Point::new(1.0, 5.0));
        assert!(empty.is_ordered());
        assert!(empty.is_empty());

        let unordered = Rect::new(Point::new(2.0, 0.0), Point::new(1.0, 5.0));
        assert!(!unordered.is_ordered());
    }

    /// Intersection: overlap is the inner rectangle; disjoint inputs give an empty
    /// result, which is the admits-nothing clip value.
    #[test]
    fn intersection_overlap_and_disjoint() {
        let a = Rect::new(Point::new(0.0, 0.0), Point::new(10.0, 10.0));
        let b = Rect::new(Point::new(5.0, -2.0), Point::new(15.0, 7.0));
        let inner = a.intersection(b);
        assert_eq!(
            inner,
            Rect::new(Point::new(5.0, 0.0), Point::new(10.0, 7.0))
        );

        let far = Rect::new(Point::new(20.0, 20.0), Point::new(30.0, 30.0));
        assert!(a.intersection(far).is_empty());
    }

    /// The recogniser accepts every closure style of a four-corner axis-aligned
    /// outline, in either edge phase and winding, and recognises the degenerate case.
    #[test]
    fn axis_aligned_rect_recognises_rectangles() {
        let closed = [
            Segment::MoveTo(Point::new(1.0, 2.0)),
            Segment::LineTo(Point::new(5.0, 2.0)),
            Segment::LineTo(Point::new(5.0, 8.0)),
            Segment::LineTo(Point::new(1.0, 8.0)),
            Segment::Close,
        ];
        let expected = Rect::new(Point::new(1.0, 2.0), Point::new(5.0, 8.0));
        assert_eq!(axis_aligned_rect(&closed), Some(expected));

        // Closed by returning to the start, no Close.
        let returned = [
            Segment::MoveTo(Point::new(5.0, 8.0)),
            Segment::LineTo(Point::new(1.0, 8.0)),
            Segment::LineTo(Point::new(1.0, 2.0)),
            Segment::LineTo(Point::new(5.0, 2.0)),
            Segment::LineTo(Point::new(5.0, 8.0)),
        ];
        assert_eq!(axis_aligned_rect(&returned), Some(expected));

        // Vertical-first phase (starts along x = 1).
        let vertical_first = [
            Segment::MoveTo(Point::new(1.0, 2.0)),
            Segment::LineTo(Point::new(1.0, 8.0)),
            Segment::LineTo(Point::new(5.0, 8.0)),
            Segment::LineTo(Point::new(5.0, 2.0)),
            Segment::Close,
        ];
        assert_eq!(axis_aligned_rect(&vertical_first), Some(expected));

        // Degenerate: zero width. Recognised — as a clip it admits nothing.
        let degenerate = [
            Segment::MoveTo(Point::new(3.0, 0.0)),
            Segment::LineTo(Point::new(3.0, 4.0)),
            Segment::LineTo(Point::new(3.0, 4.0)),
            Segment::LineTo(Point::new(3.0, 0.0)),
            Segment::Close,
        ];
        assert!(axis_aligned_rect(&degenerate).is_some());
    }

    /// Oblique edges, curves and extra corners are not rectangles.
    #[test]
    fn axis_aligned_rect_rejects_non_rectangles() {
        let oblique = [
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(4.0, 1.0)),
            Segment::LineTo(Point::new(4.0, 5.0)),
            Segment::LineTo(Point::new(0.0, 4.0)),
            Segment::Close,
        ];
        assert_eq!(axis_aligned_rect(&oblique), None);

        let curved = [
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::CubicTo {
                c1: Point::new(1.0, 0.0),
                c2: Point::new(3.0, 0.0),
                to: Point::new(4.0, 0.0),
            },
            Segment::LineTo(Point::new(4.0, 4.0)),
            Segment::LineTo(Point::new(0.0, 4.0)),
            Segment::Close,
        ];
        assert_eq!(axis_aligned_rect(&curved), None);

        let pentagon = [
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(4.0, 0.0)),
            Segment::LineTo(Point::new(4.0, 4.0)),
            Segment::LineTo(Point::new(2.0, 6.0)),
            Segment::LineTo(Point::new(0.0, 4.0)),
            Segment::Close,
        ];
        assert_eq!(axis_aligned_rect(&pentagon), None);
    }
}

//! ISO 32000-2 §8.3.3's matrix, and the four questions other subsystems ask of one.
//!
//! The six numbers are the clause's; everything else here exists because some other part
//! of the library needs one number off a transform and must ask for it the same way
//! everywhere. [`Affine::max_coefficient`] is what §4.7's coordinate bound is applied to,
//! [`Affine::preserves_axes`] is what §6.4's rectangle lane turns on,
//! [`Affine::max_stretch`] is what §6.3's atlas scale bucket is keyed by, and
//! [`Affine::invert`] refuses rather than substituting an identity.

use super::Point;

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

#[cfg(test)]
mod tests {
    use super::{Affine, Point};

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
}

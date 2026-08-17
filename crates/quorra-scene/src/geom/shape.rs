//! Where a mark is and how big: the point, the size and the axis-aligned rectangle.
//!
//! Every bound in this library — a clip, a group's extent, a device target, a shading's
//! domain — is one of these three, and none of them carries arithmetic anybody has to
//! reason about. The one operation with a clause behind it is [`Rect::intersection`],
//! because a clip chain is an intersection (§4.7 of the brief).

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

#[cfg(test)]
mod tests {
    use super::{Point, Rect};

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
}

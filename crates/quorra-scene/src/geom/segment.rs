//! The one step an outline is made of, and the one shape a run of them is recognised as.
//!
//! [`Segment`] is the vocabulary — move, line, cubic, close, and deliberately nothing
//! else (§1.1 of the brief). [`axis_aligned_rect`] is the *recogniser* §6.4 of the brief
//! asks for, and it is a decision about which lane a mark takes rather than a property of
//! a curve: the caller's display list has no rectangle type, so a rectangular clip has to
//! be found in a sequence of segments before it can become four floats instead of a mask.
//! It sits here, with the type it reads, rather than with the [`Rect`] it returns.

use super::{Point, Rect};

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
    use super::{Point, Rect, Segment, axis_aligned_rect};

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

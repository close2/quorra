//! Flattening: an outline's segments, under a transform, become device-space
//! polylines — and how finely.
//!
//! One thing: the conversion from `quorra_scene`'s curves to the straight-line
//! approximation everything downstream in [`raster`](super) works on. Nothing here
//! knows what a coverage byte is; [`fill`](super::fill) does, and it is handed
//! [`Polyline`]s.
//!
//! The bound is ISO 32000-2 §10.7.2's flatness tolerance, read twice — as a distance
//! in device pixels ([`FLATTEN_TOLERANCE`]) and as a fraction of the curve's own size
//! ([`RELATIVE_FLATTEN_TOLERANCE`], ADR 0044) — and the tighter of the two binds.
//! Subdivision is at `t = 1/2`, which is exact in `f32`, so a flattening is the same
//! on every adapter and in every thread (ADR 0008's determinism, §4.6).

use quorra_scene::{Point, Segment};

/// Maximum distance, in device pixels, between a cubic and its flattening. 0.25 px
/// keeps the flattening error below half of one coverage step at the edge of a
/// pixel; the choice is recorded in ADR 0008 with its cost.
///
/// This is the bound ISO 32000-2 §10.7.2 states:
///
/// > The flatness tolerance controls the maximum permitted distance in device pixels
/// > between the mathematically correct path and an approximation constructed from
/// > straight line segments
///
/// — measured in device pixels, which is why it is applied after the transform. The
/// same clause's "PDF processors may choose to ignore any flatness tolerance specified
/// within a PDF file" is why the number is ours and not the document's.
pub(crate) const FLATTEN_TOLERANCE: f32 = 0.25;

/// The same distance as a fraction of the cubic's own device extent — the bound that
/// binds once a whole curve is no bigger than a few [`FLATTEN_TOLERANCE`]s.
///
/// A distance in device pixels says nothing about a shape smaller than itself: at a
/// quarter pixel a circle of diameter 1 flattens to four chords and deposits its
/// **inscribed square**, 36.3 % short of its own area. §10.7.2's NOTE 2 is explicit
/// that this is not what the tolerance is for:
///
/// > the purpose of the flatness tolerance is to control the precision of curve
/// > rendering, not to draw inscribed polygons. If the parameter's value is large
/// > enough to cause visible straight line segments to appear, the result is
/// > unpredictable.
///
/// 1/32 of the curve's own control-polygon diagonal holds any closed curve to at least
/// 16 chords per full turn, whose area is `(16/2π)·sin(2π/16) = 0.9745` of the circle's
/// — 2.55 % short at worst, against the 1–4 % that rounding coverage to a byte already
/// costs a mark of that size. It never loosens the absolute bound and therefore never
/// removes a segment: no circle of radius 2.4 device pixels or more changes at all.
/// ADR 0044 has the arithmetic and the rejected alternative.
pub(crate) const RELATIVE_FLATTEN_TOLERANCE: f32 = 0.031_25;

/// A device-space transform applied during flattening: the composed
/// command-times-viewport affine, as six f32s (kept away from `quorra_scene::Affine`
/// only to avoid a needless dependency direction — the arithmetic is §8.3.3's).
#[derive(Debug, Clone, Copy)]
pub(crate) struct DeviceTransform {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl DeviceTransform {
    fn apply(self, p: Point) -> Point {
        Point::new(
            self.a * p.x + self.c * p.y + self.e,
            self.b * p.x + self.d * p.y + self.f,
        )
    }
}

/// One flattened subpath: device-space points, and whether the source closed it.
#[derive(Debug, Clone)]
pub(crate) struct Polyline {
    pub points: Vec<Point>,
    pub closed: bool,
}

/// Flatten an outline under a transform into polylines, one per subpath.
///
/// Curves subdivide at their midpoint until the control points sit within
/// [`cubic_tolerance`] of the chord — the standard flatness bound: for a cubic,
/// the curve deviates from the chord by at most 3/4 of the larger control-point
/// distance, so testing the controls bounds the curve. Subdivision at t = 1/2 is
/// exact f32 arithmetic (halving), keeping flattening deterministic everywhere.
pub(crate) fn flatten(segments: &[Segment], transform: DeviceTransform) -> Vec<Polyline> {
    let mut subpaths = Vec::new();
    let mut current: Vec<Point> = Vec::new();
    let mut push_current = |current: &mut Vec<Point>, closed: bool| {
        if current.len() > 1 {
            subpaths.push(Polyline {
                points: std::mem::take(current),
                closed,
            });
        } else {
            current.clear();
        }
    };
    for segment in segments {
        match *segment {
            Segment::MoveTo(p) => {
                push_current(&mut current, false);
                current.push(transform.apply(p));
            }
            Segment::LineTo(p) => {
                if !current.is_empty() {
                    current.push(transform.apply(p));
                }
            }
            Segment::CubicTo { c1, c2, to } => {
                if let Some(&from) = current.last() {
                    let (c1, c2, to) = (
                        transform.apply(c1),
                        transform.apply(c2),
                        transform.apply(to),
                    );
                    // Measured once for the whole cubic and carried down the
                    // subdivision, not recomputed per half: the bound is "within a
                    // fraction of *this curve*", and a bound that shrank with every
                    // split would be a fixed chord angle applied to shapes of every
                    // size. Per cubic rather than per outline for the opposite reason —
                    // one path can hold a page border and a one-pixel dot, and the dot
                    // must not inherit the border's extent.
                    let tolerance = cubic_tolerance(from, c1, c2, to);
                    flatten_cubic(from, c1, c2, to, tolerance, 0, &mut current);
                }
            }
            Segment::Close => {
                push_current(&mut current, true);
            }
        }
    }
    push_current(&mut current, false);
    subpaths
}

/// The flatness bound for one cubic: the tighter of the device tolerance and
/// [`RELATIVE_FLATTEN_TOLERANCE`] of the curve's own control-polygon diagonal
/// (ADR 0044).
///
/// The diagonal rather than the chord, because a cubic's chord can be zero while the
/// curve is not (a closed loop), and the control polygon contains the curve. Non-finite
/// coordinates cannot reach here — `Resources::upload_outline` refuses them and
/// `SceneBuilder` refuses a non-finite transform — but `min` would fall back to the
/// absolute bound if one ever did, which is the behaviour this had before.
pub(super) fn cubic_tolerance(p0: Point, p1: Point, p2: Point, p3: Point) -> f32 {
    let width = p0.x.max(p1.x).max(p2.x).max(p3.x) - p0.x.min(p1.x).min(p2.x).min(p3.x);
    let height = p0.y.max(p1.y).max(p2.y).max(p3.y) - p0.y.min(p1.y).min(p2.y).min(p3.y);
    FLATTEN_TOLERANCE.min(RELATIVE_FLATTEN_TOLERANCE * width.hypot(height))
}

fn flatten_cubic(
    p0: Point,
    p1: Point,
    p2: Point,
    p3: Point,
    tolerance: f32,
    depth: u8,
    out: &mut Vec<Point>,
) {
    // Flat when both controls are within tolerance of the chord: the curve is
    // bounded by the control polygon's deviation.
    let flat = {
        let dx = p3.x - p0.x;
        let dy = p3.y - p0.y;
        let d1 = ((p1.x - p0.x) * dy - (p1.y - p0.y) * dx).abs();
        let d2 = ((p2.x - p0.x) * dy - (p2.y - p0.y) * dx).abs();
        let len_sq = dx * dx + dy * dy;
        // Degenerate chord: fall back to control-point distance from p0.
        if len_sq <= f32::EPSILON {
            let c1 = (p1.x - p0.x).abs().max((p1.y - p0.y).abs());
            let c2 = (p2.x - p0.x).abs().max((p2.y - p0.y).abs());
            c1.max(c2) <= tolerance
        } else {
            (d1.max(d2)) * (d1.max(d2)) <= tolerance * tolerance * len_sq
        }
    };
    // The depth cap bounds work on hostile geometry; at 16 the segments are 2^-16 of
    // the curve and far below any tolerance a finite target can observe. The relative
    // bound cannot approach it from below: a control point is never further from the
    // chord than the control polygon's diagonal, and each split divides that distance
    // by about four, so 1/32 of the diagonal is reached in three levels whatever the
    // curve's size.
    if flat || depth >= 16 {
        out.push(p3);
        return;
    }
    let mid = |a: Point, b: Point| Point::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
    let q0 = mid(p0, p1);
    let q1 = mid(p1, p2);
    let q2 = mid(p2, p3);
    let r0 = mid(q0, q1);
    let r1 = mid(q1, q2);
    let split = mid(r0, r1);
    flatten_cubic(p0, q0, r0, split, tolerance, depth.saturating_add(1), out);
    flatten_cubic(split, r1, q2, p3, tolerance, depth.saturating_add(1), out);
}

/// The integer-pixel bounding box of a set of polylines, or `None` when empty.
pub(crate) fn polyline_bounds(polylines: &[Polyline]) -> Option<(f32, f32, f32, f32)> {
    let mut bounds: Option<(f32, f32, f32, f32)> = None;
    for polyline in polylines {
        for p in &polyline.points {
            bounds = Some(match bounds {
                None => (p.x, p.y, p.x, p.y),
                Some((x0, y0, x1, y1)) => (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
            });
        }
    }
    bounds
}

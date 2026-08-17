//! §10.7.2 and ADR 0044: **how finely a curve becomes chords, and what that costs its ink.**
//!
//! Every case here is a statement about the flatness bound. Two of the three read it out of
//! [`fill`](crate::raster::fill)'s coverage bytes, because ink is the only place a chord
//! that left the curve becomes visible — the bytes are the instrument, the bound is the
//! subject.

use quorra_scene::{Point, Segment};

use crate::raster::flatten::{FLATTEN_TOLERANCE, cubic_tolerance};
use crate::raster::{Rule, fill_mask, flatten};

use super::IDENTITY;

/// `4(√2 − 1)/3`: the control offset, in units of the radius, of the four-cubic
/// circle — the construction the caller's shared crate uses for §8.5.3.2's dot, and
/// the one the tolerance below is stated against.
const CIRCLE_K: f32 = 0.552_284_8;

/// A circle of radius `r` about `(cx, cy)`, as the four cubics a document draws it
/// with.
fn circle_path(cx: f32, cy: f32, r: f32) -> Vec<Segment> {
    let k = CIRCLE_K * r;
    let p = |x: f32, y: f32| Point::new(cx + x, cy + y);
    vec![
        Segment::MoveTo(p(r, 0.0)),
        Segment::CubicTo {
            c1: p(r, k),
            c2: p(k, r),
            to: p(0.0, r),
        },
        Segment::CubicTo {
            c1: p(-k, r),
            c2: p(-r, k),
            to: p(-r, 0.0),
        },
        Segment::CubicTo {
            c1: p(-r, -k),
            c2: p(-k, -r),
            to: p(0.0, -r),
        },
        Segment::CubicTo {
            c1: p(k, -r),
            c2: p(r, -k),
            to: p(r, 0.0),
        },
        Segment::Close,
    ]
}

/// The most a flattened closed curve may fall short of its own area, as a fraction
/// of it (ADR 0044).
///
/// `RELATIVE_FLATTEN_TOLERANCE` holds any full turn to at least 16 chords, and a
/// regular 16-gon inscribed in a circle covers `(16/2π)·sin(2π/16) = 0.974495` of
/// it. Derived, not observed: the flattener is free to place *more* chords than the
/// bound requires, and a 16-gon is the most area 16 points on a circle can enclose,
/// so no flattening of a circle may be further short than this.
const MAX_AREA_DEFICIT: f32 = 1.0 - 0.974_495;

/// A cubic with collinear control points is a straight line: flattening must not
/// bend it, so the fill equals the `LineTo` version byte for byte.
#[test]
fn collinear_cubic_equals_the_line() {
    let with_cubic = vec![
        Segment::MoveTo(Point::new(0.0, 0.0)),
        Segment::LineTo(Point::new(6.0, 0.0)),
        Segment::CubicTo {
            c1: Point::new(6.0, 2.0),
            c2: Point::new(6.0, 4.0),
            to: Point::new(6.0, 6.0),
        },
        Segment::LineTo(Point::new(0.0, 6.0)),
        Segment::Close,
    ];
    let with_line = vec![
        Segment::MoveTo(Point::new(0.0, 0.0)),
        Segment::LineTo(Point::new(6.0, 0.0)),
        Segment::LineTo(Point::new(6.0, 6.0)),
        Segment::LineTo(Point::new(0.0, 6.0)),
        Segment::Close,
    ];
    let a = fill_mask(&flatten(&with_cubic, IDENTITY), Rule::NonZero, 0, 0, 7, 7);
    let b = fill_mask(&flatten(&with_line, IDENTITY), Rule::NonZero, 0, 0, 7, 7);
    assert_eq!(a.coverage, b.coverage);
}

/// **A circle deposits its own area at every size, including the sub-pixel ones.**
///
/// The caller's `QUORRA_FEEDBACK.md` §21.2: at diameters 0.5, 1.0 and 2.0 device
/// pixels this rasteriser deposited 36.1 %, 36.1 % and 10.1 % less ink than
/// `π·r²` — the inscribed square, the inscribed square, and the inscribed octagon,
/// which is what a quarter-pixel flatness bound admits when a whole curve is a
/// pixel across. ISO 32000-2 §10.7.2's NOTE 2 says what the bound is for: "the
/// purpose of the flatness tolerance is to control the precision of curve
/// rendering, not to draw inscribed polygons".
///
/// The bound compared against is two terms, both arithmetic:
///
/// - [`MAX_AREA_DEFICIT`], the 16-chord polygon's shortfall, one-sided because a
///   chord never leaves the curve's convex hull;
/// - one half of a coverage step for each pixel the circle can touch, either way,
///   because coverage is quantised by `round(cov × 255)` at every one of them.
#[test]
fn a_circle_deposits_its_own_area_at_every_size() {
    // A pixel centre, so a mark of any diameter is centred in the grid rather than
    // straddling it: the touched-pixel count below is then the honest one.
    const C: f32 = 4.5;
    for diameter in [0.5_f32, 1.0, 2.0] {
        let r = diameter / 2.0;
        let mask = fill_mask(
            &flatten(&circle_path(C, C, r), IDENTITY),
            Rule::NonZero,
            0,
            0,
            9,
            9,
        );
        let ink: f32 = mask.coverage.iter().map(|b| f32::from(*b) / 255.0).sum();
        let area = std::f32::consts::PI * r * r;

        // Only pixels the circle reaches carry a rounding error; the rest are an
        // exact zero. `[floor(c − r), ceil(c + r))` is §10.7.4's half-open pixel
        // rule applied to the mark's own bounds.
        let touched = (C + r).ceil() - (C - r).floor();
        let quantum = touched * touched * 0.5 / 255.0;

        assert!(
            ink <= area + quantum,
            "a circle of diameter {diameter} drew {ink:.4}, more than its area \
             {area:.4} plus {quantum:.4} of rounding — a chord left the curve"
        );
        assert!(
            ink >= area * (1.0 - MAX_AREA_DEFICIT) - quantum,
            "a circle of diameter {diameter} drew {ink:.4} against its area \
             {area:.4}: {:.2}% short, past the {:.2}% a 16-chord flattening and \
             {quantum:.4} of rounding allow",
            100.0 * (area - ink) / area,
            100.0 * MAX_AREA_DEFICIT,
        );
    }
}

/// **The relative bound is inert on anything big, and that is a perf statement.**
///
/// `RELATIVE_FLATTEN_TOLERANCE` enters through a `min`, so it can only ever add
/// segments, and it adds none once `extent/32 ≥ 0.25` — a control polygon 8 device
/// pixels across, which for a circle's quarter-arc cubic (diagonal `r√2`) is a
/// radius of 5.66. The population that pays for ADR 0044 is therefore bounded by
/// arithmetic rather than by hope, and the counts below pin it.
///
/// A quarter-arc of half-angle `α` puts its controls `(4/3)·r·tan(α/2)·sin(α)` from
/// its chord, so at `r = 20` the successive depths are 7.81, 2.03, 0.51 and 0.128
/// device pixels: three splits, eight chords a quarter, **32 a turn**. The polyline
/// carries one point more than that because it opens on the `MoveTo` and closes by
/// returning to it.
#[test]
fn a_large_curve_keeps_the_segment_count_it_had() {
    let big = flatten(&circle_path(40.0, 40.0, 20.0), IDENTITY);
    assert_eq!(big.len(), 1, "one subpath");
    assert_eq!(big[0].points.len(), 33, "32 chords a turn at r = 20");

    // The `min` is not binding anywhere on that curve: every one of its four cubics
    // spans `r√2 = 28.3` device pixels, and 28.3/32 is past a quarter pixel.
    for segment in circle_path(40.0, 40.0, 20.0) {
        if let Segment::CubicTo { c1, c2, to } = segment {
            let from = Point::new(40.0 + 20.0, 40.0);
            assert!(
                cubic_tolerance(from, c1, c2, to) >= FLATTEN_TOLERANCE,
                "the relative bound must not tighten a 28-pixel cubic"
            );
        }
    }

    // And the small ones are where it does bind: 16 chords a turn, at every size.
    for diameter in [0.5_f32, 1.0, 2.0, 4.0] {
        let small = flatten(&circle_path(8.0, 8.0, diameter / 2.0), IDENTITY);
        assert_eq!(
            small[0].points.len(),
            17,
            "a circle of diameter {diameter} flattens to 16 chords"
        );
    }
}

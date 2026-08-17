//! §8.4.3: **caps, joins, and the expansion's arithmetic at the ends of the coordinate
//! range.**
//!
//! Every case here is a statement about the stroke clause, and every one of them is read
//! out of [`fill`](crate::raster::fill)'s coverage bytes — because
//! [`stroke`](crate::raster::stroke) takes polylines and returns polylines, so filling the
//! polygons it expands is the only way to observe them. That is the reason ADR 0061 gave
//! for keeping these beside the fill cases; it is a fact about the *instrument*, and the
//! subject of `each_cap_deposits_the_area_table_53_gives_it` is Table 53 rather than
//! §8.5.3.3's rules.
//!
//! The last two cover `stroke::direction` and `stroke_polylines`' coincident-point
//! test — the defects the parent's module comment attributes to this part, both of which
//! drew a wrong page and neither of which any page-sized fixture could reach.

use quorra_scene::{LineCap, LineJoin, Point, Segment, Stroke};

use crate::raster::{Polyline, Rule, fill_mask, flatten, stroke_polylines};

use super::{IDENTITY, cov, rect_path};

/// A horizontal stroke of width 2 with butt caps is exactly a rectangle: the
/// expansion must produce the same bytes as filling that rectangle directly.
#[test]
fn butt_stroke_of_a_horizontal_line_is_a_rectangle() {
    let line = vec![
        Segment::MoveTo(Point::new(2.0, 3.0)),
        Segment::LineTo(Point::new(8.0, 3.0)),
    ];
    let stroke = Stroke {
        width: 2.0,
        cap: LineCap::Butt,
        join: LineJoin::Miter,
        miter_limit: 10.0,
    };
    let stroked = stroke_polylines(&flatten(&line, IDENTITY), stroke);
    let a = fill_mask(&stroked, Rule::NonZero, 0, 0, 10, 6);
    let b = fill_mask(
        &flatten(&rect_path(2.0, 2.0, 8.0, 4.0), IDENTITY),
        Rule::NonZero,
        0,
        0,
        10,
        6,
    );
    assert_eq!(a.coverage, b.coverage);
}

/// A square cap extends the same rectangle by half the width at each end.
#[test]
fn square_caps_extend_by_half_the_width() {
    let line = vec![
        Segment::MoveTo(Point::new(2.0, 3.0)),
        Segment::LineTo(Point::new(8.0, 3.0)),
    ];
    let stroke = Stroke {
        width: 2.0,
        cap: LineCap::Square,
        join: LineJoin::Miter,
        miter_limit: 10.0,
    };
    let stroked = stroke_polylines(&flatten(&line, IDENTITY), stroke);
    let a = fill_mask(&stroked, Rule::NonZero, 0, 0, 10, 6);
    let b = fill_mask(
        &flatten(&rect_path(1.0, 2.0, 9.0, 4.0), IDENTITY),
        Rule::NonZero,
        0,
        0,
        10,
        6,
    );
    assert_eq!(a.coverage, b.coverage);
}

/// A right-angle miter join fills the outer corner square: the L-shaped stroke
/// equals the union of its two rectangles plus that corner, all derivable.
#[test]
fn miter_join_fills_the_corner() {
    let l_path = vec![
        Segment::MoveTo(Point::new(2.0, 2.0)),
        Segment::LineTo(Point::new(8.0, 2.0)),
        Segment::LineTo(Point::new(8.0, 8.0)),
    ];
    let stroke = Stroke {
        width: 2.0,
        cap: LineCap::Butt,
        join: LineJoin::Miter,
        miter_limit: 10.0,
    };
    let stroked = stroke_polylines(&flatten(&l_path, IDENTITY), stroke);
    let mask = fill_mask(&stroked, Rule::NonZero, 0, 0, 11, 11);
    // The outer corner pixel (9 - epsilon region: x in 8..9, y in 1..2) is inside
    // the miter; with a bevel it would be half-covered at best.
    assert_eq!(cov(&mask, 8, 1), 255, "miter fills the outer corner");
    // Body of each arm.
    assert_eq!(cov(&mask, 4, 1), 255);
    assert_eq!(cov(&mask, 4, 2), 255);
    assert_eq!(cov(&mask, 8, 5), 255);
    // Outside the stroke.
    assert_eq!(cov(&mask, 4, 4), 0);
    assert_eq!(cov(&mask, 1, 1), 0);
}

/// **Every cap deposits the area Table 53 gives it** (ISO 32000-2 §8.4.3.3), at both
/// ends of the subpath.
///
/// The expectations are the clause's own arithmetic on a `length × width` rule, not
/// anything read off this rasteriser:
///
/// - **Butt** — "the stroke shall be squared off at the endpoint": `length × width`.
/// - **Round** — "[a] semicircular arc with a diameter equal to the line width shall
///   be drawn around the endpoint and shall be filled in": two half-discs of radius
///   `width / 2`, so `length × width + π × width² / 4`.
/// - **Projecting square** — "the stroke shall continue beyond the endpoint for a
///   distance equal to half the line width": `(length + width) × width`.
///
/// Round was the butt figure **to four decimals** before this test existed, and the
/// reason is worth keeping beside the numbers: the cap at the far end was a correct
/// outward semicircle and the cap at the near end was the *inward* one, wound against
/// the body it lay inside, so the non-zero rule cancelled them to the texel. A total
/// that agrees is not a picture that agrees — this test measures the near end and the
/// far end separately for that reason.
#[test]
fn each_cap_deposits_the_area_table_53_gives_it() {
    // The rule runs from x = 20 to x = 60 at y = 40, well inside an 80 × 80 mask so
    // that no cap is clipped by the raster's edge. The column bounds below are those
    // two numbers, as integers.
    const X0: f32 = 20.0;
    const LENGTH: f32 = 40.0;
    const WIDTH: f32 = 5.0;
    let ink = |cap: LineCap, left: i32, width: u32| -> f32 {
        let y = 40.0;
        let line = Polyline {
            points: vec![Point::new(X0, y), Point::new(X0 + LENGTH, y)],
            closed: false,
        };
        let stroke = Stroke {
            width: WIDTH,
            cap,
            join: LineJoin::Round,
            miter_limit: 10.0,
        };
        let mask = fill_mask(
            &stroke_polylines(&[line], stroke),
            Rule::NonZero,
            left,
            0,
            width,
            80,
        );
        mask.coverage.iter().map(|b| f32::from(*b) / 255.0).sum()
    };
    // A quarter of a texel: the sampling grid's own resolution, not a fitted bound.
    let close = |got: f32, want: f32, what: &str| {
        assert!(
            (got - want).abs() < 0.25,
            "{what}: {got:.4} against the clause's {want:.4}"
        );
    };

    let body = LENGTH * WIDTH;
    close(ink(LineCap::Butt, 0, 80), body, "butt");
    close(
        ink(LineCap::Square, 0, 80),
        (LENGTH + WIDTH) * WIDTH,
        "projecting square",
    );
    let discs = std::f32::consts::PI * WIDTH * WIDTH / 4.0;
    close(ink(LineCap::Round, 0, 80), body + discs, "round");

    // And each end on its own, because the defect this replaces was two errors of
    // equal size in opposite directions: the near end must *gain* a half-disc over a
    // butt cap, not lose one.
    let half = discs / 2.0;
    // Columns 0..20 hold only what lies left of the rule's start, and 60..80 only
    // what lies right of its end.
    close(ink(LineCap::Butt, 0, 20), 0.0, "butt, near end");
    close(ink(LineCap::Butt, 60, 20), 0.0, "butt, far end");
    close(ink(LineCap::Round, 0, 20), half, "round, near end");
    close(ink(LineCap::Round, 60, 20), half, "round, far end");
}

/// The largest device coordinate the scene contract admits, derived rather than
/// picked: `MAX_COORDINATE` is `1e9` on an outline point *and* on each of a command
/// transform's and the viewport transform's coefficients, so
/// `point × command × viewport` reaches `1e27` and a sum of two such terms `2e27`.
///
/// `Device::render` refuses a viewport transform above `MAX_COORDINATE`
/// (`RenderError::ViewportTransformTooLarge`), which is what makes this a *bound*
/// rather than a guess.
const LARGEST_DEVICE_COORDINATE: f32 = 2e27;

fn hairline() -> Stroke {
    Stroke {
        width: 4.0,
        cap: LineCap::Butt,
        join: LineJoin::Miter,
        miter_limit: 10.0,
    }
}

/// **A stroke at the top of the coordinate range still deposits its ink.**
///
/// `direction`'s `dx * dx` overflows to infinity above `1.9e19`, which is eight
/// orders of magnitude below what the contract admits; the length was then infinite,
/// the normal `(0, 0)`, and the stroke's quad had no width at all. A mark asked for
/// and drawn as nothing is §5's forbidden third state, and no test could see it
/// because every fixture in the tree is a page-sized number.
#[test]
fn a_stroke_spanning_the_coordinate_range_is_not_drawn_as_nothing() {
    let line = Polyline {
        points: vec![
            Point::new(0.0, 4.0),
            Point::new(LARGEST_DEVICE_COORDINATE, 4.0),
        ],
        closed: false,
    };
    let expanded = stroke_polylines(&[line], hairline());
    assert!(
        expanded
            .iter()
            .flat_map(|line| &line.points)
            .all(|p| p.x.is_finite() && p.y.is_finite()),
        "no piece of the expansion may be non-finite: {expanded:?}"
    );
    let mask = fill_mask(&expanded, Rule::NonZero, 0, 0, 8, 8);
    assert_eq!(cov(&mask, 4, 3), 255, "the band's own row is covered");
    assert_eq!(cov(&mask, 4, 4), 255, "and so is the other side of it");
    assert_eq!(cov(&mask, 4, 0), 0, "four pixels above it is not");
}

/// **A segment too short for the float grid produces no NaN geometry**, which is
/// CLAUDE.md's rule about a document's numbers stated at the one place in the tree
/// that broke it.
///
/// `stroke_polylines` drops coincident neighbours by comparing coordinates for
/// equality, and two points `1e-30` apart are not equal — while `dx * dx` for that
/// delta underflows to zero, so the length was zero and the normal `(NaN, ±inf)`.
/// Both coordinates are inside `MAX_COORDINATE`, so this is an outline a document may
/// state.
#[test]
fn a_segment_below_the_float_grid_produces_finite_geometry() {
    let line = Polyline {
        points: vec![Point::new(0.0, 4.0), Point::new(1e-30, 4.0)],
        closed: false,
    };
    let expanded = stroke_polylines(&[line], hairline());
    assert!(
        expanded
            .iter()
            .flat_map(|line| &line.points)
            .all(|p| p.x.is_finite() && p.y.is_finite()),
        "a piece of the expansion is not finite: {expanded:?}"
    );
    let mask = fill_mask(&expanded, Rule::NonZero, 0, 0, 8, 8);
    assert!(
        mask.coverage.iter().all(|&byte| byte == 0),
        "a butt-capped stroke of a zero-length segment covers nothing: {:?}",
        mask.coverage
    );
}

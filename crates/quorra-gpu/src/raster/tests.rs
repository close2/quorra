//! What the three parts of [`raster`](super) are asked to produce, on shapes whose
//! answer is derivable by hand or by a clause.
//!
//! One file rather than one per part, because almost every case here goes through all
//! three: a stroke's caps are measured by *filling* the polygons
//! [`stroke`](super::stroke) expands, and a circle's area is a statement about
//! [`flatten`](mod@super::flatten) read out of [`fill`](super::fill)'s bytes. The seams
//! the source is split along are not the seams a coverage byte can be observed at.
#![allow(clippy::arithmetic_side_effects)] // test indices are tiny and literal

use quorra_scene::{LineCap, LineJoin, Point, Segment, Stroke};

// `FLATTEN_TOLERANCE` and `cubic_tolerance` come from the module that owns them rather
// than through a re-export: nothing outside `raster` asks for either, so re-exporting
// them from the parent would widen them for this file's benefit alone.
use super::flatten::{FLATTEN_TOLERANCE, cubic_tolerance};
use super::{CoverageMask, DeviceTransform, Polyline, Rule, fill_mask, flatten, stroke_polylines};

const IDENTITY: DeviceTransform = DeviceTransform {
    a: 1.0,
    b: 0.0,
    c: 0.0,
    d: 1.0,
    e: 0.0,
    f: 0.0,
};

fn cov(mask: &CoverageMask, x: usize, y: usize) -> u8 {
    mask.coverage[y * mask.width as usize + x]
}

fn rect_path(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<Segment> {
    vec![
        Segment::MoveTo(Point::new(x0, y0)),
        Segment::LineTo(Point::new(x1, y0)),
        Segment::LineTo(Point::new(x1, y1)),
        Segment::LineTo(Point::new(x0, y1)),
        Segment::Close,
    ]
}

/// A pixel-aligned rectangle: full coverage inside, zero outside — the module's
/// definition evaluated where it is derivable by inspection.
#[test]
fn aligned_rectangle_is_exact() {
    let polylines = flatten(&rect_path(1.0, 1.0, 5.0, 4.0), IDENTITY);
    let mask = fill_mask(&polylines, Rule::NonZero, 0, 0, 6, 5);
    assert_eq!(cov(&mask, 0, 0), 0);
    assert_eq!(cov(&mask, 2, 2), 255);
    assert_eq!(cov(&mask, 4, 3), 255);
    assert_eq!(cov(&mask, 5, 4), 0);
    assert_eq!(cov(&mask, 1, 0), 0);
}

/// Fractional edges: a rectangle from 0.5 to 2.5 covers its edge pixels by
/// exactly half — round(0.5 × 255) = 128 by the stated quantisation.
#[test]
fn fractional_rectangle_covers_halves() {
    let polylines = flatten(&rect_path(0.5, 0.0, 2.5, 1.0), IDENTITY);
    let mask = fill_mask(&polylines, Rule::NonZero, 0, 0, 3, 1);
    assert_eq!(cov(&mask, 0, 0), 128);
    assert_eq!(cov(&mask, 1, 0), 255);
    assert_eq!(cov(&mask, 2, 0), 128);
}

/// A right triangle covering exactly half of a 4x4 box: the diagonal pixels get
/// half coverage, one side full, the other empty — areas derivable by hand.
#[test]
fn diagonal_covers_by_area() {
    let tri = vec![
        Segment::MoveTo(Point::new(0.0, 0.0)),
        Segment::LineTo(Point::new(4.0, 0.0)),
        Segment::LineTo(Point::new(0.0, 4.0)),
        Segment::Close,
    ];
    let polylines = flatten(&tri, IDENTITY);
    let mask = fill_mask(&polylines, Rule::NonZero, 0, 0, 4, 4);
    // On the diagonal, each pixel's covered area is exactly half its cell.
    for i in 0..4 {
        assert_eq!(cov(&mask, i, 3 - i), 128, "diagonal pixel ({i}, {})", 3 - i);
    }
    // Fully inside and fully outside.
    assert_eq!(cov(&mask, 0, 0), 255);
    assert_eq!(cov(&mask, 3, 3), 0);
}

/// §4.7's discriminating case: a nested subpath wound the *same* way. Non-zero
/// fills the hole; even-odd leaves it empty. Expected values follow from the two
/// rules' definitions in §8.5.3.3.
#[test]
fn nested_same_winding_separates_the_rules() {
    let mut path = rect_path(0.0, 0.0, 8.0, 8.0);
    path.extend(rect_path(2.0, 2.0, 6.0, 6.0)); // same winding order
    let polylines = flatten(&path, IDENTITY);
    let nonzero = fill_mask(&polylines, Rule::NonZero, 0, 0, 8, 8);
    let evenodd = fill_mask(&polylines, Rule::EvenOdd, 0, 0, 8, 8);
    assert_eq!(cov(&nonzero, 4, 4), 255, "non-zero fills the nested area");
    assert_eq!(cov(&evenodd, 4, 4), 0, "even-odd holes the nested area");
    assert_eq!(cov(&nonzero, 1, 1), 255);
    assert_eq!(cov(&evenodd, 1, 1), 255);
}

/// Winding from geometry outside the mask region still counts: rasterising only
/// a window into a larger rectangle fills the window completely.
#[test]
fn geometry_outside_the_region_still_winds() {
    let polylines = flatten(&rect_path(-100.0, -100.0, 100.0, 100.0), IDENTITY);
    let mask = fill_mask(&polylines, Rule::NonZero, 10, 10, 4, 4);
    for y in 0..4 {
        for x in 0..4 {
            assert_eq!(cov(&mask, x, y), 255);
        }
    }
}

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

/// **A tile is the crop of any region that contains it** (ADR 0049) — the property
/// the residue cache rests on, stated where the arithmetic that has to hold it is.
///
/// Forty closed curves of the artwork archetype's shape, each rasterised once over
/// its own bounds and then twenty times over tiles cut out of it at arbitrary
/// offsets, including tiles that hang off every side. Every pixel of every tile must
/// read what the region reads at the same *device* pixel — the region and the tile
/// share the pixel grid, so this compares like with like.
///
/// **The bound is not zero, and the reason is arithmetic rather than geometry.** The
/// two are the same sum of the same trapezoids in a different order: the region's
/// prefix sum crosses the columns left of the tile one at a time where the tile
/// takes them as a single deposit at its border column. `f32` addition is not
/// associative, so a value within about 1e-7 of a rounding step can land either side
/// of it. What the assertions below fix is that this — and nothing structural — is
/// all that is left: **31 pixels of 2 863 228, every one of them by 1 of 255**.
/// Before ADR 0049's border cut the same probe read 2 684 pixels and **185 of 255**,
/// which is what a smeared border column looks like from here.
// A probe over generated geometry: the casts below are between pixel indices and
// device coordinates that the loops keep inside the region, and the arithmetic is
// the probe's own bookkeeping.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::arithmetic_side_effects
)]
#[test]
fn a_tile_is_the_crop_of_the_region_that_contains_it() {
    let mut state: u32 = 0x1234_5678;
    let mut next = |bound: f32| -> f32 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((state >> 8) as f32 / 16_777_216.0) * bound
    };
    let mut worst = 0i32;
    let mut differing = 0u64;
    let mut total = 0u64;
    for shape in 0..40 {
        // A closed curve like the archetype's clip: 24 cubics about a centre.
        let cx = 60.0 + next(1000.0);
        let cy = 60.0 + next(1500.0);
        let r = 30.0 + next(120.0);
        let mut path = vec![Segment::MoveTo(Point::new(cx - r, cy))];
        let steps = 24;
        for step in 0..steps {
            let from = (step as f32) / (steps as f32) * std::f32::consts::TAU;
            let to = ((step + 1) as f32) / (steps as f32) * std::f32::consts::TAU;
            let point = |angle: f32| Point::new(cx + r * angle.cos(), cy + r * angle.sin() * 1.3);
            let (a, b) = (point(from), point(to));
            path.push(Segment::CubicTo {
                c1: Point::new(a.x + (b.x - a.x) * 0.35, a.y + (b.y - a.y) * 0.1),
                c2: Point::new(a.x + (b.x - a.x) * 0.65, a.y + (b.y - a.y) * 0.9),
                to: b,
            });
        }
        path.push(Segment::Close);
        let lines = flatten(&path, IDENTITY);
        let (x0, y0, x1, y1) = super::polyline_bounds(&lines).unwrap();
        let (rl, rt) = (x0.floor() as i32, y0.floor() as i32);
        let (rw, rh) = (
            (x1.ceil() as i32 - rl) as u32,
            (y1.ceil() as i32 - rt) as u32,
        );
        let region = fill_mask(&lines, Rule::NonZero, rl, rt, rw, rh);
        for _ in 0..20 {
            let tl = rl + next(rw as f32) as i32 - 20;
            let tt = rt + next(rh as f32) as i32 - 20;
            let tw = 20 + next(80.0) as u32;
            let th = 20 + next(80.0) as u32;
            let direct = fill_mask(&lines, Rule::NonZero, tl, tt, tw, th);
            for y in 0..th as i32 {
                for x in 0..tw as i32 {
                    let d = direct.coverage[(y * tw as i32 + x) as usize];
                    let (gx, gy) = (tl + x - rl, tt + y - rt);
                    let c = if gx < 0 || gy < 0 || gx >= rw as i32 || gy >= rh as i32 {
                        0
                    } else {
                        region.coverage[(gy * rw as i32 + gx) as usize]
                    };
                    total += 1;
                    if c != d {
                        differing += 1;
                        worst = worst.max((i32::from(c) - i32::from(d)).abs());
                    }
                }
            }
        }
        let _ = shape;
    }
    assert!(
        worst <= 1,
        "a tile differs from its region by {worst} of 255, which is not rounding: \
         {differing} of {total} pixels differ"
    );
    assert!(
        differing * 50_000 <= total,
        "{differing} of {total} pixels differ — 31 of 2 863 228 is what rounding \
         costs here, and an order of magnitude more than that is a structural \
         difference wearing rounding's clothes"
    );
}

/// **A tile whose geometry enters from outside gets the area, not a smear of it**
/// (ADR 0049), against a value derived from the geometry rather than from any
/// rasteriser.
///
/// The shape's left boundary is one straight edge from `(−2, 0)` to `(2, 1)`; the
/// interior is to its right, and the tile is the single pixel `[0, 1] × [0, 1]`. The
/// edge crosses `x = 0` at `y = 0.5` and `x = 1` at `y = 0.75`, so of that pixel:
///
/// - `y ∈ [0, 0.5]` — the boundary is left of the pixel, which is covered: **0.5**
/// - `y ∈ [0.5, 0.75]` — the boundary crosses it; the area to its right is
///   `∫(1 − 4(y − 0.5)) dy = 0.25 − 0.125` = **0.125**
/// - `y ∈ [0.75, 1]` — the boundary is right of the pixel, which is empty: **0**
///
/// 0.625 of the pixel, and `round(0.625 × 255) = 159` by this module's stated
/// quantisation. Clamping the endpoints instead of cutting at the border spread the
/// slab evenly across `x ∈ [0, 1]` and read **128**.
#[test]
fn a_tile_whose_geometry_enters_from_outside_is_exact() {
    let path = vec![
        Segment::MoveTo(Point::new(-2.0, 0.0)),
        Segment::LineTo(Point::new(2.0, 1.0)),
        Segment::LineTo(Point::new(9.0, 1.0)),
        Segment::LineTo(Point::new(9.0, 0.0)),
        Segment::Close,
    ];
    let lines = flatten(&path, IDENTITY);
    let tile = fill_mask(&lines, Rule::NonZero, 0, 0, 1, 1);
    assert_eq!(cov(&tile, 0, 0), 159, "0.625 of a pixel is 159 of 255");
}

/// The same shape's pixel, asked for as part of a wider region: the two agree, which
/// is [`a_tile_is_the_crop_of_the_region_that_contains_it`] on a case whose answer is
/// known independently of either.
#[test]
fn the_region_and_the_tile_agree_on_a_pixel_whose_area_is_known() {
    let path = vec![
        Segment::MoveTo(Point::new(-2.0, 0.0)),
        Segment::LineTo(Point::new(2.0, 1.0)),
        Segment::LineTo(Point::new(9.0, 1.0)),
        Segment::LineTo(Point::new(9.0, 0.0)),
        Segment::Close,
    ];
    let lines = flatten(&path, IDENTITY);
    let region = fill_mask(&lines, Rule::NonZero, -4, 0, 16, 1);
    assert_eq!(cov(&region, 4, 0), 159, "the pixel at device x = 0");
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

/// **An edge whose slope leaves `f32` deposits nothing, not a solid row.**
///
/// The triangle below is `1e-30` of a pixel tall and `1e9` wide — both inside
/// `MAX_COORDINATE`, so a document may state it. Its slope is `1e39`, which is
/// infinite in `f32`; the slab arithmetic then produced a NaN, the prefix sum carried
/// it to the end of the row, and `abs().min(1.0)` returns **1.0** for a NaN — so one
/// invisible sliver painted its whole row solid.
///
/// Zero is not a compromise here but the exact answer to eleven decimal places: the
/// area such an edge covers is under `1e-30` where one coverage step is `1/255`.
#[test]
fn an_edge_whose_slope_leaves_f32_deposits_nothing() {
    let sliver = Polyline {
        points: vec![
            Point::new(0.0, 1e-30),
            Point::new(1e9, 2e-30),
            Point::new(0.0, 2e-30),
        ],
        closed: true,
    };
    let mask = fill_mask(&[sliver], Rule::NonZero, 0, 0, 8, 8);
    assert!(
        mask.coverage.iter().all(|&byte| byte == 0),
        "a sliver 1e-30 of a pixel tall covers no pixel: {:?}",
        mask.coverage
    );
}

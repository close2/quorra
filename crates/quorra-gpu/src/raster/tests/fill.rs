//! §8.5.3.3 with ADR 0005 and ADR 0049: **polylines into coverage bytes over a region** —
//! the accumulation grid, the two fill rules, and a region's cut at a tile's border.
//!
//! Every case here is a statement about what a coverage byte *is*: the area rule the
//! module documents, the non-zero and even-odd rules read off one shape that separates
//! them, and the property ADR 0049's residue cache rests on. The last of them,
//! [`an_edge_whose_slope_leaves_f32_deposits_nothing`], covers `fill::accumulate_edge` —
//! one of the two defects the parent's module comment attributes to this part.

use quorra_scene::{Point, Segment};

use crate::raster::{Polyline, Rule, fill_mask, flatten, polyline_bounds};

use super::{IDENTITY, cov, rect_path};

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
        let (x0, y0, x1, y1) = polyline_bounds(&lines).unwrap();
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

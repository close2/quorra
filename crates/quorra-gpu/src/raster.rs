//! CPU coverage rasterisation: the one producer of coverage bytes for the glyph and
//! path lanes.
//!
//! # Why the CPU, and why our own (ADR 0008)
//!
//! The glyph atlas (M4) and the general path lane (M5) both need an R8 coverage
//! image of an outline. Rasterising it on the CPU makes the bytes **identical on
//! every adapter** — coverage becomes immune to the driver-owned float→unorm
//! conversion that ADR 0006 measured, leaving only the final blend inside the ±1
//! bound — and it needs no compute pipeline on the startup path. Writing our own
//! (rather than depending on `tiny-skia`) keeps the dependency graph as `deny.toml`
//! wants it and keeps the arithmetic *stated*: every rule below is a documented
//! choice a test can derive expectations from, because ISO 32000-2 does not define
//! anti-aliasing (ADR 0005 records that silence).
//!
//! # The definition of coverage
//!
//! Flattened edges deposit exact signed trapezoid areas into an accumulation grid;
//! a left-to-right prefix sum recovers the average winding `w` per pixel; coverage is
//! - non-zero rule (ISO 32000-2 §8.5.3.3.2): `min(|w|, 1)`,
//! - even-odd rule (§8.5.3.3.3): `1 − |1 − (w mod 2)|`, the triangle fold, which
//!   agrees with the parity of the winding number wherever a pixel is crossed by a
//!   single edge and is our stated behaviour where several cross one pixel.
//!
//! Curves flatten by recursive midpoint subdivision to a stated tolerance — the tighter
//! of [`FLATTEN_TOLERANCE`] and [`RELATIVE_FLATTEN_TOLERANCE`] of the curve's own size
//! (ADR 0044); strokes expand to closed polygons (§8.4.3's caps and joins) and fill
//! non-zero. Quantisation to a byte is `round(cov × 255)`.

use quorra_scene::{LineCap, LineJoin, Point, Segment, Stroke};

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

/// A rasterised coverage tile: `width × height` bytes anchored at integer device
/// pixel `(left, top)`.
#[derive(Debug, Clone)]
pub(crate) struct CoverageMask {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    /// Row-major coverage bytes, `width × height`.
    pub coverage: Vec<u8>,
}

impl CoverageMask {
    /// A mask that admits nothing, over the given pixels.
    ///
    /// A legitimate mask rather than the absence of one: an empty clip region admits
    /// nothing *inside* it too, which is a different statement from having no clip, and
    /// both have tests (`doc/PLAN.md` §1.4).
    pub(crate) fn transparent(left: i32, top: i32, width: u32, height: u32) -> Self {
        Self {
            left,
            top,
            width,
            height,
            coverage: vec![0; (width as usize).saturating_mul(height as usize)],
        }
    }

    /// The window of this mask over another rectangle of device pixels, transparent
    /// wherever the two do not meet.
    ///
    /// **This is only a lookup because [`fill_mask`] cuts at its region's border** — the
    /// two share the pixel grid, and the same device pixel carries the same coverage
    /// whichever region computed it, to within the 1-of-255 rounding
    /// `a_tile_is_the_crop_of_the_region_that_contains_it` bounds. Outside this mask the
    /// answer is transparent by construction: a region is the intersection of its
    /// chain's bounds, and a closed path winds nothing beyond its own.
    // The corners are computed in `i64`, where an `i32` origin plus a `u32` extent cannot
    // wrap; every offset below is then a difference between two of those corners inside
    // the overlap, so it is non-negative and no larger than the smaller mask's extent.
    // Stated once here rather than at each of the six.
    #[allow(clippy::arithmetic_side_effects)]
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    pub(crate) fn crop(&self, left: i32, top: i32, width: u32, height: u32) -> Self {
        let mut cut = Self::transparent(left, top, width, height);
        let (ax0, ay0) = (i64::from(left), i64::from(top));
        let (ax1, ay1) = (ax0 + i64::from(width), ay0 + i64::from(height));
        let (bx0, by0) = (i64::from(self.left), i64::from(self.top));
        let (bx1, by1) = (bx0 + i64::from(self.width), by0 + i64::from(self.height));
        let (x0, y0) = (ax0.max(bx0), ay0.max(by0));
        let (x1, y1) = (ax1.min(bx1), ay1.min(by1));
        if x0 >= x1 || y0 >= y1 {
            return cut;
        }
        let (span, rows) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let (from_x, from_y) = ((x0 - bx0) as usize, (y0 - by0) as usize);
        let (into_x, into_y) = ((x0 - ax0) as usize, (y0 - ay0) as usize);
        for row in 0..rows {
            let from = (from_y + row) * self.width as usize + from_x;
            let into = (into_y + row) * width as usize + into_x;
            cut.coverage[into..into + span].copy_from_slice(&self.coverage[from..from + span]);
        }
        cut
    }
}

/// Which of ISO 32000-2 §8.5.3.3's two rules decides insideness.
///
/// `Hash` because it is part of the glyph cache's key: the same outline under the two
/// rules is two different pictures wherever a subpath nests (ADR 0024).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Rule {
    NonZero,
    EvenOdd,
}

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
fn cubic_tolerance(p0: Point, p1: Point, p2: Point, p3: Point) -> f32 {
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

/// Rasterise closed polylines into a coverage mask over the given integer pixel
/// region (`left..left+width`, `top..top+height`), by the module's stated definition.
///
/// Every subpath is treated as closed (fill semantics, ISO 32000-2 §8.5.3.1: filling
/// implicitly closes open subpaths).
///
/// **The region is a window on one answer, not an answer of its own.** Geometry outside
/// it is cut at the border and deposits its winding there ([`deposit_slab`]), so asking
/// for a tighter region returns what a wider one holds over the same pixels — to within
/// the accumulator's own rounding, which ADR 0049 measures at **1 of 255 on 2 pixels in
/// 2.9 million**. That is what lets one rasterisation of a clip's region serve every
/// tile cut out of it (`encode::residue`).
// The accumulation arithmetic below is bounded by construction: coordinates are
// clamped into the region, whose dimensions were checked against the frame budget
// before allocation. Stated once here rather than per line of a hot loop.
#[allow(clippy::arithmetic_side_effects)]
pub(crate) fn fill_mask(
    polylines: &[Polyline],
    rule: Rule,
    left: i32,
    top: i32,
    width: u32,
    height: u32,
) -> CoverageMask {
    let w = width as usize;
    let h = height as usize;
    // One spill column: a deposit at the right edge lands in it rather than wrapping.
    let mut acc = vec![0.0_f32; (w + 1) * h];

    #[allow(clippy::cast_precision_loss)] // region dims are bounded by target limits
    let (fw, fh) = (w as f32, h as f32);
    for polyline in polylines {
        let n = polyline.points.len();
        for i in 0..n {
            let p0 = polyline.points[i];
            // Filling closes every subpath: the last edge returns to the start.
            let p1 = polyline.points[(i + 1) % n];
            #[allow(clippy::cast_precision_loss)]
            let (x0, y0) = (p0.x - left as f32, p0.y - top as f32);
            #[allow(clippy::cast_precision_loss)]
            let (x1, y1) = (p1.x - left as f32, p1.y - top as f32);
            accumulate_edge(&mut acc, w, fw, fh, x0, y0, x1, y1);
        }
    }

    // Prefix-sum each row: the running total is the average winding per pixel; the
    // rule maps winding to coverage; `round` quantises (our stated rule, ADR 0005).
    let mut coverage = vec![0_u8; w * h];
    for y in 0..h {
        let mut running = 0.0_f32;
        for x in 0..w {
            running += acc[y * (w + 1) + x];
            let cov = match rule {
                Rule::NonZero => running.abs().min(1.0),
                Rule::EvenOdd => {
                    let m = running.abs().rem_euclid(2.0);
                    1.0 - (m - 1.0).abs()
                }
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                coverage[y * w + x] = (cov * 255.0).round() as u8;
            }
        }
    }
    CoverageMask {
        left,
        top,
        width,
        height,
        coverage,
    }
}

/// Deposit one edge's signed trapezoid areas into the accumulation grid.
///
/// The edge is split at every horizontal pixel row and every vertical pixel column
/// it crosses, so each piece lies within one cell; a piece from `(xs, ys)` to
/// `(xe, ye)` inside cell `k` deposits `d·(1 − xm)` into `k` and `d·xm` into `k+1`,
/// where `d` is the signed slab height and `xm` the piece's mean x within the cell —
/// the exact trapezoid area to the right of the edge, plus the spill that keeps the
/// running sum equal to the full winding beyond the crossing.
#[allow(clippy::arithmetic_side_effects, clippy::cast_possible_truncation)]
#[allow(clippy::cast_sign_loss, clippy::cast_precision_loss)]
// Two endpoints plus the grid: the coordinate bundle is the function's whole input,
// and a struct would only rename the eight numbers.
#[allow(clippy::too_many_arguments)]
fn accumulate_edge(
    acc: &mut [f32],
    w: usize,
    fw: f32,
    fh: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
) {
    // Exact comparison: a horizontal edge deposits nothing by definition, and a
    // nearly-horizontal one deposits its nearly-zero area correctly.
    #[allow(clippy::float_cmp)]
    if y0 == y1 {
        return;
    }
    let (dir, top_x, top_y, bot_x, bot_y) = if y0 < y1 {
        (1.0_f32, x0, y0, x1, y1)
    } else {
        (-1.0, x1, y1, x0, y0)
    };
    // Clip vertically to the region; x interpolates along the clipped span.
    let (top_x, top_y) = if top_y < 0.0 {
        (
            top_x + (bot_x - top_x) * (0.0 - top_y) / (bot_y - top_y),
            0.0,
        )
    } else {
        (top_x, top_y)
    };
    let (bot_x, bot_y) = if bot_y > fh {
        (top_x + (bot_x - top_x) * (fh - top_y) / (bot_y - top_y), fh)
    } else {
        (bot_x, bot_y)
    };
    if bot_y <= top_y {
        return;
    }
    let dxdy = (bot_x - top_x) / (bot_y - top_y);
    // **A slope this edge cannot state is a slab this edge cannot fill.** The numerator
    // is bounded by twice the largest device coordinate the scene contract admits
    // (`MAX_COORDINATE` on a point and on a transform coefficient, so `4e27`), and the
    // denominator is positive by the test above — so a non-finite ratio means the slab
    // is under `2.4e-11` of a pixel tall, and the exact area such an edge deposits is
    // under `2.4e-11` where one coverage step is `1/255`. Depositing nothing is the
    // right answer to eleven decimal places, and it is the only answer that keeps a NaN
    // out of the accumulator: a NaN survives the prefix sum, and `abs().min(1.0)`
    // returns **1.0** for it, so one such edge paints the rest of its row solid. The
    // same test also catches a NaN arriving from a coordinate that is not finite, which
    // `Device::render` refuses at the viewport before it can reach here.
    if !dxdy.is_finite() {
        return;
    }

    let mut y = top_y.floor().max(0.0);
    while y < bot_y {
        let row = y as usize;
        if row >= acc.len() / (w + 1) {
            break;
        }
        let entry_y = top_y.max(y);
        let exit_y = bot_y.min(y + 1.0);
        let entry_x = top_x + (entry_y - top_y) * dxdy;
        let exit_x = top_x + (exit_y - top_y) * dxdy;
        deposit_slab(
            &mut acc[row * (w + 1)..(row + 1) * (w + 1)],
            fw,
            dir,
            entry_x,
            entry_y,
            exit_x,
            exit_y,
        );
        y += 1.0;
    }
}

/// Deposit one row slab's areas. `xs`/`xe` are x at the slab's top and bottom; the part
/// of the slab spent left or right of the region is **cut off at the border** and
/// deposited there, rather than compressed into the columns inside it.
///
/// # Why the cut, and what clamping the endpoints instead used to cost (ADR 0049)
///
/// A slab piece running from `x = −25` to `x = +2` covers the region's first column for
/// most of its height and only reaches `x = 2` at the very end. Clamping the two
/// endpoints to `[0, fw]` and interpolating between them — which is what this function
/// did until ADR 0049 — spreads that height evenly from column 0 to column 2 instead.
/// The row's *total* winding survives (every column past the crossing reads the same
/// value, which is why nothing downstream ever saw it), but the columns at the border
/// get somebody else's share: **up to 185 of 255 on a shallow edge**, measured by
/// `a_tile_whose_geometry_enters_from_outside_is_exact`.
///
/// Cutting at the border is the same statement §10.7.4 makes about a clipping region —
/// the pixels a fill would cover — applied to the region this mask is asked for: what
/// lies outside contributes its winding, at the border, for exactly the height it spends
/// there.
///
/// The cut runs only when an endpoint is outside; a piece wholly inside takes the same
/// arithmetic it always did, to the bit, which is what keeps every tile that is not cut
/// by a clip or by the page edge pixel-for-pixel where it was.
#[allow(clippy::arithmetic_side_effects, clippy::cast_possible_truncation)]
#[allow(clippy::cast_sign_loss, clippy::cast_precision_loss)]
fn deposit_slab(row: &mut [f32], fw: f32, dir: f32, xs: f32, ys: f32, xe: f32, ye: f32) {
    if xs >= 0.0 && xs <= fw && xe >= 0.0 && xe <= fw {
        deposit_inside(row, fw, dir, xs, ys, xe, ye);
        return;
    }
    let (dx, dy) = (xe - xs, ye - ys);
    // At most two borders can be crossed, and `dx == 0` crosses neither: a vertical
    // piece is on one side for its whole height.
    let mut cuts = [(0.0_f32, 0.0_f32); 2];
    let mut count = 0;
    if dx != 0.0 {
        for border in [0.0_f32, fw] {
            let t = (border - xs) / dx;
            if t > 0.0 && t < 1.0 {
                cuts[count] = (t, border);
                count += 1;
            }
        }
        if count == 2 && cuts[1].0 < cuts[0].0 {
            cuts.swap(0, 1);
        }
    }
    // Each part is interpolated from the piece's own ends, so a cut cannot move where
    // the piece starts or finishes: the border's own x is used at the seam, and the
    // outer ends stay the values the caller passed.
    let (mut px, mut py) = (xs, ys);
    for (t, border) in cuts.iter().take(count).copied() {
        let (nx, ny) = (border, ys + dy * t);
        deposit_inside(row, fw, dir, px, py, nx, ny);
        (px, py) = (nx, ny);
    }
    deposit_inside(row, fw, dir, px, py, xe, ye);
}

/// One slab piece that does not cross the region's borders: split at each vertical cell
/// boundary it does cross, and deposit the exact trapezoid areas.
///
/// A piece wholly outside arrives here with both ends on the same side; the clamp then
/// collapses it onto the border column, which is where its winding belongs.
#[allow(clippy::arithmetic_side_effects, clippy::cast_possible_truncation)]
#[allow(clippy::cast_sign_loss, clippy::cast_precision_loss)]
fn deposit_inside(row: &mut [f32], fw: f32, dir: f32, xs: f32, ys: f32, xe: f32, ye: f32) {
    let xs = xs.clamp(0.0, fw);
    let xe = xe.clamp(0.0, fw);
    let (mut px, mut py) = (xs, ys);
    loop {
        // The next vertical boundary in the direction of travel, or the slab's end.
        let boundary = if xe > px {
            let b = px.floor() + 1.0;
            if b < xe { Some(b) } else { None }
        } else if xe < px {
            let b = px.ceil() - 1.0;
            if b > xe { Some(b) } else { None }
        } else {
            None
        };
        let (nx, ny) = match boundary {
            Some(b) => {
                let t = (b - xs) / (xe - xs);
                (b, ys + (ye - ys) * t)
            }
            None => (xe, ye),
        };
        // One single-cell piece: exact trapezoid deposit.
        let d = dir * (ny - py);
        if d != 0.0 {
            let xm = 0.5 * (px + nx);
            let cell = (xm.floor().max(0.0) as usize).min(row.len().saturating_sub(2));
            let frac = xm - cell as f32;
            row[cell] += d * (1.0 - frac);
            row[cell + 1] += d * frac;
        }
        if boundary.is_none() {
            break;
        }
        (px, py) = (nx, ny);
    }
}

/// Expand a stroke into closed polygons (ISO 32000-2 §8.4.3: quads per segment,
/// §8.4.3.4's joins at interior vertices, §8.4.3.3's caps at open ends), for filling
/// with the non-zero rule — overlaps between pieces double the winding, which
/// non-zero coverage clamps away.
///
/// Widths arrive resolved and positive, dashing already applied, degenerate subpaths
/// pre-split (§4.5 of the brief); consecutive coincident points are skipped here so
/// flattening artefacts cannot produce zero-length pieces.
#[allow(clippy::arithmetic_side_effects)]
pub(crate) fn stroke_polylines(polylines: &[Polyline], stroke: Stroke) -> Vec<Polyline> {
    let hw = stroke.width * 0.5;
    let mut out = Vec::new();
    for polyline in polylines {
        // Dedupe coincident neighbours (and the closing wrap, when closed).
        let mut pts: Vec<Point> = Vec::with_capacity(polyline.points.len());
        for &p in &polyline.points {
            #[allow(clippy::float_cmp)] // exact: a zero-length piece, not a near one
            if pts.last().is_none_or(|q| q.x != p.x || q.y != p.y) {
                pts.push(p);
            }
        }
        #[allow(clippy::float_cmp)]
        if polyline.closed
            && pts.len() > 1
            && pts[0].x == pts[pts.len() - 1].x
            && pts[0].y == pts[pts.len() - 1].y
        {
            pts.pop();
        }
        if pts.len() < 2 {
            continue; // a lone point: degenerate, pre-split upstream (§8.5.3.2)
        }

        let segment_count = if polyline.closed {
            pts.len()
        } else {
            pts.len() - 1
        };
        // One quad per segment.
        for i in 0..segment_count {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            let n = normal(a, b, hw);
            out.push(Polyline {
                points: vec![
                    Point::new(a.x + n.x, a.y + n.y),
                    Point::new(b.x + n.x, b.y + n.y),
                    Point::new(b.x - n.x, b.y - n.y),
                    Point::new(a.x - n.x, a.y - n.y),
                ],
                closed: true,
            });
        }
        // Joins at interior vertices (all vertices when closed).
        let join_count = if polyline.closed {
            pts.len()
        } else {
            pts.len().saturating_sub(2)
        };
        for j in 0..join_count {
            let prev = pts[j];
            let v = pts[(j + 1) % pts.len()];
            let next = pts[(j + 2) % pts.len()];
            join_at(&mut out, prev, v, next, hw, stroke.join, stroke.miter_limit);
        }
        // Caps at open ends.
        if !polyline.closed {
            let first_dir = direction(pts[0], pts[1]);
            let last = pts.len() - 1;
            let last_dir = direction(pts[last - 1], pts[last]);
            cap_at(
                &mut out,
                pts[0],
                Point::new(-first_dir.x, -first_dir.y),
                hw,
                stroke.cap,
            );
            cap_at(&mut out, pts[last], last_dir, hw, stroke.cap);
        }
    }
    out
}

/// The unit vector from `a` to `b`, or the zero vector when there is no direction to
/// give — which is a piece the caller has already established is not zero-length.
///
/// # Why the second computation exists
///
/// `dx * dx` is not the length of anything on its own, and it leaves the representable
/// range at both ends long before `dx` does: it overflows to infinity above `1.9e19`
/// and underflows to zero below `1.1e-22`. **Both ends are inside the scene contract.**
/// `MAX_COORDINATE` is `1e9` on an outline point *and* on a transform coefficient, and a
/// device delta is a composed transform applied to such a point, so `dx` reaches `1e27`;
/// at the other end nothing bounds a delta from below except the float grid, and
/// `stroke_polylines`' dedupe compares coordinates for *equality*, which two points
/// `1e-30` apart pass.
///
/// Neither end used to be caught, and neither failed quietly at the same place:
/// - `1e27` gave `len = inf`, so the normal was `(0, 0)`, so the stroke's quad was
///   degenerate and the mark **deposited no ink at all**;
/// - `1e-30` gave `len = 0`, so the normal was `(NaN, ±inf)`, and `fill_mask`'s prefix
///   sum carries a NaN to the end of its row where `abs().min(1.0)` returns **1.0** for
///   it — a fully painted row across the tile. CLAUDE.md's rule is that a document's
///   numbers must "never produce NaN geometry"; this was the one place in the tree that
///   did.
///
/// `hypot` is the same length without the square and is exact at both ends. It is the
/// *second* path rather than the only one because it is a libm call on the hottest
/// stroke loop there is — the caller's reading of hayro #630 is the standing warning
/// about exactly that — and because every segment the fast path already handles must
/// keep the arithmetic it had, to the bit, so that no page of the corpus moves.
fn direction(a: Point, b: Point) -> Point {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len.is_finite() && len > 0.0 {
        return Point::new(dx / len, dy / len);
    }
    let len = dx.hypot(dy);
    if len.is_finite() && len > 0.0 {
        Point::new(dx / len, dy / len)
    } else {
        // `a` and `b` are the same point to the last bit, or one of them is not finite —
        // neither of which `stroke_polylines` hands us, since it dedupes exact repeats
        // and its input is bounded by `MAX_COORDINATE`. A zero direction makes every
        // piece built from it degenerate, which deposits nothing (§8.5.3.2's degenerate
        // subpath), rather than letting a NaN into the accumulator.
        Point::new(0.0, 0.0)
    }
}

/// The left normal of `a → b`, scaled to the half-width.
fn normal(a: Point, b: Point, hw: f32) -> Point {
    let d = direction(a, b);
    Point::new(-d.y * hw, d.x * hw)
}

#[allow(clippy::arithmetic_side_effects)]
fn join_at(
    out: &mut Vec<Polyline>,
    prev: Point,
    v: Point,
    next: Point,
    hw: f32,
    join: LineJoin,
    miter_limit: f32,
) {
    let d1 = direction(prev, v);
    let d2 = direction(v, next);
    let cross = d1.x * d2.y - d1.y * d2.x;
    if cross == 0.0 {
        return; // collinear: the quads already meet edge-to-edge
    }
    // The gap opens on the side away from the turn.
    let s = if cross > 0.0 { -1.0 } else { 1.0 };
    let n1 = Point::new(-d1.y * hw * s, d1.x * hw * s);
    let n2 = Point::new(-d2.y * hw * s, d2.x * hw * s);
    let p1 = Point::new(v.x + n1.x, v.y + n1.y);
    let p2 = Point::new(v.x + n2.x, v.y + n2.y);
    match join {
        LineJoin::Bevel => out.push(Polyline {
            points: vec![v, p1, p2],
            closed: true,
        }),
        LineJoin::Miter => {
            // §8.4.3.5: the miter stands until length/width exceeds the limit; then
            // the join is a bevel. Ratio = 1 / cos(half-angle), via the unit normals.
            let dot = (n1.x * n2.x + n1.y * n2.y) / (hw * hw);
            let denom = 1.0 + dot;
            let ratio_sq = 2.0 / denom.max(f32::EPSILON);
            if ratio_sq <= miter_limit * miter_limit {
                let scale = 1.0 / denom.max(f32::EPSILON);
                let m = Point::new(v.x + (n1.x + n2.x) * scale, v.y + (n1.y + n2.y) * scale);
                out.push(Polyline {
                    points: vec![v, p1, m, p2],
                    closed: true,
                });
            } else {
                out.push(Polyline {
                    points: vec![v, p1, p2],
                    closed: true,
                });
            }
        }
        LineJoin::Round => {
            out.push(arc_fan(v, p1, p2, hw));
        }
    }
}

#[allow(clippy::arithmetic_side_effects)]
fn cap_at(out: &mut Vec<Polyline>, end: Point, dir: Point, hw: f32, cap: LineCap) {
    // `dir` points outward, away from the stroked segment.
    let n = Point::new(-dir.y * hw, dir.x * hw);
    match cap {
        LineCap::Butt => {}
        LineCap::Square => out.push(Polyline {
            points: vec![
                Point::new(end.x + n.x, end.y + n.y),
                Point::new(end.x + n.x + dir.x * hw, end.y + n.y + dir.y * hw),
                Point::new(end.x - n.x + dir.x * hw, end.y - n.y + dir.y * hw),
                Point::new(end.x - n.x, end.y - n.y),
            ],
            closed: true,
        }),
        // §8.4.3.3, Table 53: "[a] semicircular arc with a diameter equal to the line
        // width shall be drawn around the endpoint and shall be filled in."
        LineCap::Round => out.push(cap_fan(end, dir, hw)),
    }
}

/// The semicircle a round cap is, swept from one side of the stroke round **through
/// `dir`** to the other — `dir` pointing away from the segment, as [`cap_at`] takes it.
///
/// Built from `dir` rather than from the two endpoint angles, and that is the whole
/// point. A cap sweeps **exactly pi**, and an arc stated by its endpoints alone has two
/// readings at exactly pi that no "shorter way round" rule can separate: [`arc_fan`] took
/// whichever way `atan2`'s branch cut happened to give, which was the outward semicircle
/// at the end of a subpath and the *inward* one at its start.
///
/// An inward semicircle is not merely invisible. It lies inside the stroke body and is
/// wound **against** it, so the non-zero rule (§10.7.4's fill, `fill_mask`) cancels the
/// two and punches a hole of exactly the area the far cap adds. Both ends of every
/// round-capped subpath were wrong, and the two errors were equal and opposite: the
/// caller's ink-total instrument read a round cap as depositing exactly what a butt cap
/// does (`QUORRA_FEEDBACK.md` §21.1), which is the sum, not the picture.
///
/// The step count is [`ARC_STEP`]'s, as for any other arc.
#[allow(clippy::arithmetic_side_effects)]
fn cap_fan(end: Point, dir: Point, hw: f32) -> Polyline {
    // `cap_at`'s `n` is `dir` turned a quarter turn, so the cap's two corners sit at
    // `base ± pi/2` and the outward point at `base`. Sweeping downward from `+pi/2`
    // passes through `base`, which is what makes this the outward half — and gives the
    // fan the stroke body's own winding, so it adds rather than cancels.
    let base = dir.y.atan2(dir.x);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // pi / ARC_STEP
    let steps = ((std::f32::consts::PI / ARC_STEP).ceil() as usize).max(1);
    let mut points = Vec::with_capacity(steps.saturating_add(2));
    points.push(end);
    for i in 0..=steps {
        #[allow(clippy::cast_precision_loss)] // steps is a dozen
        let t =
            base + std::f32::consts::FRAC_PI_2 - std::f32::consts::PI * (i as f32) / (steps as f32);
        points.push(Point::new(end.x + hw * t.cos(), end.y + hw * t.sin()));
    }
    Polyline {
        points,
        closed: true,
    }
}

/// The angle one step of an arc advances: deterministic (§4.6), and within
/// [`FLATTEN_TOLERANCE`] for any stroke width a page realistically holds.
const ARC_STEP: f32 = 0.35;

/// A fan of points approximating the arc from `from` to `to` around `centre` (both on
/// the circle of radius `radius`), as one closed polygon including the centre.
///
/// **The caller must guarantee a sweep of less than pi**, because that is what makes
/// "the shorter way round" below name one arc rather than two. [`join_at`] is the only
/// caller and it does: it returns before this on `cross == 0.0`, so the two segments are
/// never collinear and never a reversal, and the gap a join fills is strictly under a
/// half turn. A cap *is* exactly a half turn and has [`cap_fan`] for that reason.
#[allow(clippy::arithmetic_side_effects)]
fn arc_fan(centre: Point, from: Point, to: Point, radius: f32) -> Polyline {
    let a0 = (from.y - centre.y).atan2(from.x - centre.x);
    let a1 = (to.y - centre.y).atan2(to.x - centre.x);
    let mut sweep = a1 - a0;
    // Take the shorter way round: a join or cap never sweeps more than pi.
    if sweep > std::f32::consts::PI {
        sweep -= 2.0 * std::f32::consts::PI;
    } else if sweep < -std::f32::consts::PI {
        sweep += 2.0 * std::f32::consts::PI;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let steps = ((sweep.abs() / ARC_STEP).ceil() as usize).max(1);
    let mut points = vec![centre, from];
    for i in 1..steps {
        #[allow(clippy::cast_precision_loss)]
        let t = a0 + sweep * (i as f32) / (steps as f32);
        points.push(Point::new(
            centre.x + radius * t.cos(),
            centre.y + radius * t.sin(),
        ));
    }
    points.push(to);
    Polyline {
        points,
        closed: true,
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)] // test indices are tiny and literal
mod tests {
    use quorra_scene::{LineCap, LineJoin, Point, Segment, Stroke};

    use super::{
        CoverageMask, DeviceTransform, FLATTEN_TOLERANCE, Polyline, Rule, cubic_tolerance,
        fill_mask, flatten, stroke_polylines,
    };

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
                let point =
                    |angle: f32| Point::new(cx + r * angle.cos(), cy + r * angle.sin() * 1.3);
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
}

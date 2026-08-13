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
//! Curves flatten by recursive midpoint subdivision to a stated tolerance
//! ([`FLATTEN_TOLERANCE`]); strokes expand to closed polygons (§8.4.3's caps and
//! joins) and fill non-zero. Quantisation to a byte is `round(cov × 255)`.

use quorra_scene::{LineCap, LineJoin, Point, Segment, Stroke};

/// Maximum distance, in device pixels, between a cubic and its flattening. 0.25 px
/// keeps the flattening error below half of one coverage step at the edge of a
/// pixel; the choice is recorded in ADR 0008 with its cost.
pub(crate) const FLATTEN_TOLERANCE: f32 = 0.25;

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
/// [`FLATTEN_TOLERANCE`] of the chord — the standard flatness bound: for a cubic,
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
                    flatten_cubic(
                        from,
                        transform.apply(c1),
                        transform.apply(c2),
                        transform.apply(to),
                        0,
                        &mut current,
                    );
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

fn flatten_cubic(p0: Point, p1: Point, p2: Point, p3: Point, depth: u8, out: &mut Vec<Point>) {
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
            c1.max(c2) <= FLATTEN_TOLERANCE
        } else {
            (d1.max(d2)) * (d1.max(d2)) <= FLATTEN_TOLERANCE * FLATTEN_TOLERANCE * len_sq
        }
    };
    // The depth cap bounds work on hostile geometry; at 16 the segments are 2^-16 of
    // the curve and far below any tolerance a finite target can observe.
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
    flatten_cubic(p0, q0, r0, split, depth.saturating_add(1), out);
    flatten_cubic(split, r1, q2, p3, depth.saturating_add(1), out);
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
/// implicitly closes open subpaths). Geometry outside the region contributes its
/// winding by clamping to the region's edges, so a region tighter than the path's
/// bounds still fills correctly.
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

/// Deposit one row slab's areas, splitting at each vertical cell boundary the edge
/// crosses. `xs`/`xe` are x at the slab's top and bottom; geometry left or right of
/// the region clamps to its border (winding preserved, position clamped).
#[allow(clippy::arithmetic_side_effects, clippy::cast_possible_truncation)]
#[allow(clippy::cast_sign_loss, clippy::cast_precision_loss)]
fn deposit_slab(row: &mut [f32], fw: f32, dir: f32, xs: f32, ys: f32, xe: f32, ye: f32) {
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

fn direction(a: Point, b: Point) -> Point {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt();
    Point::new(dx / len, dy / len)
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
        CoverageMask, DeviceTransform, Polyline, Rule, fill_mask, flatten, stroke_polylines,
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
}

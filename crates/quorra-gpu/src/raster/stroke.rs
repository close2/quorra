//! Stroking: a resolved stroke becomes closed polygons, to be filled non-zero.
//!
//! One thing: ISO 32000-2 §8.4.3's geometry. **A stroke is not coverage and this
//! module produces none** — it takes [`Polyline`]s from [`flatten`](mod@super::flatten)
//! and hands back [`Polyline`]s, one closed polygon per segment quad, per join
//! (§8.4.3.4) and per cap (§8.4.3.3), for [`fill`](super::fill) to rasterise under
//! [`Rule::NonZero`](super::Rule). Overlaps between the pieces double the winding,
//! which non-zero coverage clamps away — so the pieces need no boolean union, and that
//! is the whole reason the expansion may be this simple.
//!
//! **Every fan here is wound the way the body it joins is**, and that invariant is
//! load-bearing rather than decorative: a fan wound against the body punches a hole of
//! exactly its own area instead of adding one, which is the defect [`cap_fan`] states
//! and `each_cap_deposits_the_area_table_53_gives_it` measures.

use quorra_scene::{LineCap, LineJoin, Point, Stroke};

use super::flatten::Polyline;

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
/// [`FLATTEN_TOLERANCE`](super::flatten::FLATTEN_TOLERANCE) for any stroke width a page
/// realistically holds.
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

//! Outlines as triangles: the geometry half of the GPU coverage lane (ADR 0016).
//!
//! Evan Wallace's *Easy Scalable Text Rendering on the GPU* (2016) turns a filled
//! outline into triangles that a rasteriser can accumulate:
//!
//! - pick one anchor point, and for every segment emit the triangle
//!   `(anchor, from, to)` — the **chord fan**. Summed with a sign per triangle
//!   orientation, those triangles cover a point exactly `winding` times, which is
//!   ISO 32000-2 §8.5.3.3's winding number and nothing else;
//! - for every *curved* segment emit one more triangle over its control polygon, whose
//!   fragments are kept only where they fall inside the curve — Loop and Blinn's test.
//!   That triangle adds the bulge between chord and curve, or subtracts it, purely by
//!   its own orientation. Nothing here has to know which.
//!
//! The whole point is that **no step depends on the device scale**: this module runs
//! once per outline at upload, and a frame at 100× re-uses exactly what a frame at 1×
//! did. That is the difference from `raster.rs`, whose flattening tolerance is in
//! device pixels and whose cost therefore grows with magnification.
//!
//! # Cubics, and why they become quadratics here
//!
//! Loop and Blinn's implicit test is exact for a *quadratic*; the cubic case needs
//! their classification into serpentines, loops and cusps, with an orientation flip
//! that a filled path then has to reconcile. A PDF outline is cubic (§8.5.2.2's `c`
//! operator), so this module converts, once, with a stated bound:
//!
//! - a cubic is within `√3/36 · |d|` of the quadratic through the same ends with
//!   control `¾(c₁ + c₂) − ¼(p₀ + p₃)`, where `d = p₃ − 3c₂ + 3c₁ − p₀` is the cubic's
//!   third difference. `d` vanishes exactly when the cubic *is* a quadratic, and
//!   splitting at `t = ½` divides it by eight — so the subdivision converges fast and
//!   terminates on the bound rather than on a depth guess.
//! - the bound is taken **relative to the outline's own extent**
//!   ([`QUAD_TOLERANCE`]), because the conversion happens before any transform exists.
//!   The device-space consequence is stated there.

use quorra_scene::{Point, Segment};

/// How far a converted quadratic may sit from the cubic it replaces, as a fraction of
/// the outline's bounding-box diagonal.
///
/// The conversion is transform-independent — that is its whole value — so the bound
/// cannot be in device pixels, and this is the honest consequence: at magnification
/// `m` the error is `1e-4 · diagonal · m` pixels. A 10-unit glyph stays inside a
/// hundredth of a pixel to 100× and inside a tenth to 1000×, which is an order of
/// magnitude past the 64× a viewer's zoom offers. Beyond that the curve is *smooth*
/// rather than wrong — an error in the geometry, never in the coverage of the geometry
/// that was submitted.
const QUAD_TOLERANCE: f32 = 1.0e-4;

/// How far a cubic may sit from its quadratic approximation, per unit of third
/// difference: `√3/36`. Loop and Blinn's bound, and the reason the subdivision below
/// terminates on geometry rather than on a depth guess.
const THIRD_DIFFERENCE_BOUND: f32 = 0.048_112_52;

/// Deepest subdivision of one cubic.
///
/// The bound terminates the recursion on its own (each split divides the third
/// difference by eight, so eight levels buy a factor of 2²⁴); this exists so that a
/// non-finite coefficient that slipped past the scene boundary cannot spin. Reaching
/// it is not an error: the quadratics emitted are still a curve, just a coarser one.
const MAX_SPLIT_DEPTH: u8 = 8;

/// The diagonal of an outline's control-point bounding box: the scale its conversion
/// tolerance is relative to.
fn segment_extent(segments: &[Segment]) -> f32 {
    let mut bounds: Option<(f32, f32, f32, f32)> = None;
    let mut extend = |p: Point| {
        bounds = Some(match bounds {
            None => (p.x, p.y, p.x, p.y),
            Some((x0, y0, x1, y1)) => (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
        });
    };
    for segment in segments {
        match *segment {
            Segment::MoveTo(p) | Segment::LineTo(p) => extend(p),
            Segment::CubicTo { c1, c2, to } => {
                extend(c1);
                extend(c2);
                extend(to);
            }
            Segment::Close => {}
        }
    }
    bounds.map_or(0.0, |(x0, y0, x1, y1)| (x1 - x0).hypot(y1 - y0))
}

/// One quadratic segment of a converted outline, in outline space.
///
/// A straight segment is `control: None` rather than a quadratic with a collinear
/// control point: the distinction survives into the triangle builder, where a line
/// costs one triangle and a curve costs two.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct QuadSegment {
    /// The control point, or `None` for a straight segment.
    pub control: Option<Point>,
    /// Where the segment ends. Its start is the previous segment's end.
    pub to: Point,
}

/// An outline as closed contours of quadratic segments — the form the GPU lane draws.
///
/// Built once per outline at upload ([`Self::from_segments`]) because nothing in it
/// depends on the transform the outline is eventually drawn under.
#[derive(Debug, Clone, Default)]
pub(crate) struct QuadOutline {
    /// Every contour, each as its start point and the segments that leave it.
    contours: Vec<(Point, Vec<QuadSegment>)>,
    /// Total segment count across contours, for sizing the vertex buffer.
    segments: usize,
}

impl QuadOutline {
    /// Converts a scene outline: cubics to quadratics, subpaths to closed contours.
    ///
    /// Closure is implicit, as filling requires (§8.5.3.1: "any subpath that is not
    /// explicitly closed shall be closed by the `f` operator before painting"), so a
    /// contour whose last point is not its first gains the closing chord here. That
    /// chord is geometry, not a repair: it is what the clause says the fill sees.
    pub(crate) fn from_segments(segments: &[Segment]) -> Self {
        // Relative to the outline's own extent, measured here rather than taken from a
        // caller: the conversion runs before any transform exists, so the bound cannot
        // be in device pixels, and an *absolute* one would give a 14-unit glyph the
        // same allowance as a 600-unit page border — thirty-two quadratics where four
        // would have been inside a thousandth of the glyph.
        let extent = segment_extent(segments);
        let tolerance = QUAD_TOLERANCE * extent.max(f32::MIN_POSITIVE);
        let mut outline = Self::default();
        let mut current: Option<(Point, Vec<QuadSegment>)> = None;
        let mut cursor = Point::new(0.0, 0.0);
        for segment in segments {
            match *segment {
                Segment::MoveTo(point) => {
                    outline.finish_contour(current.take());
                    cursor = point;
                    current = Some((point, Vec::new()));
                }
                Segment::LineTo(point) => {
                    if let Some((_, parts)) = current.as_mut() {
                        parts.push(QuadSegment {
                            control: None,
                            to: point,
                        });
                    }
                    cursor = point;
                }
                Segment::CubicTo { c1, c2, to } => {
                    if let Some((_, parts)) = current.as_mut() {
                        push_cubic(parts, cursor, c1, c2, to, tolerance, 0);
                    }
                    cursor = to;
                }
                // Closing is implicit for every contour, so the operator adds no
                // segment of its own; the chord back to the start is added on finish.
                Segment::Close => {
                    if let Some((start, _)) = current.as_ref() {
                        cursor = *start;
                    }
                }
            }
        }
        outline.finish_contour(current.take());
        outline
    }

    /// Adds a contour, closing it and dropping the ones that cannot enclose area.
    ///
    /// The rule is geometric rather than a guess: a closed contour of **straight**
    /// chords needs three of them to enclose anything, since two are a line traced out
    /// and back and their triangles cancel exactly. One curved segment, on the other
    /// hand, encloses the region between itself and its closing chord, so a two-segment
    /// contour survives whenever either segment bends. (§4.5's degenerate subpaths are
    /// the caller's decision, already taken; this is only about not emitting triangles
    /// whose sum is provably zero.)
    #[allow(clippy::float_cmp)] // exact, as `transform_preserves_axes` is: a contour is
    // closed when its last point *is* its first, and a tolerance here would close a
    // contour the caller left open by a hair and change the geometry it asked for.
    fn finish_contour(&mut self, contour: Option<(Point, Vec<QuadSegment>)>) {
        let Some((start, mut parts)) = contour else {
            return;
        };
        if let Some(last) = parts.last()
            && (last.to.x != start.x || last.to.y != start.y)
        {
            parts.push(QuadSegment {
                control: None,
                to: start,
            });
        }
        let curved = parts.iter().any(|part| part.control.is_some());
        if parts.len() < 3 && !curved {
            return;
        }
        self.segments = self.segments.saturating_add(parts.len());
        self.contours.push((start, parts));
    }

    /// What the converted form costs while resident, for the resource budget.
    pub(crate) fn stored_bytes(&self) -> u64 {
        let per_contour = size_of::<(Point, Vec<QuadSegment>)>() as u64;
        let segments = (self.segments as u64).saturating_mul(size_of::<QuadSegment>() as u64);
        segments.saturating_add((self.contours.len() as u64).saturating_mul(per_contour))
    }

    /// Whether this outline can cover anything at all.
    pub(crate) fn is_empty(&self) -> bool {
        self.contours.is_empty()
    }

    /// Triangles this outline needs: one per segment, two per curved segment.
    #[cfg(test)]
    #[allow(clippy::arithmetic_side_effects)] // a sum of two lengths of the same Vec
    pub(crate) fn triangle_count(&self) -> usize {
        self.contours
            .iter()
            .map(|(_, parts)| {
                parts.len() + parts.iter().filter(|part| part.control.is_some()).count()
            })
            .sum()
    }

    /// Appends this outline's triangles, transformed into device space.
    ///
    /// `place` maps an outline point to the device pixel it lands on; `clip` is the
    /// tile the fragments belong to, carried per vertex so that one draw call can hold
    /// every tile of a frame (the alternative is one scissored draw per tile, which is
    /// thousands of draws on a page of text).
    ///
    /// The anchor is the contour's own start point, which makes every contour's first
    /// and last chord triangle degenerate — they are emitted anyway rather than
    /// special-cased, because a zero-area triangle rasterises no fragments and the
    /// branch costs more than the vertices do.
    pub(crate) fn append_triangles(
        &self,
        place: impl Fn(Point) -> [f32; 2],
        clip: [f32; 4],
        out: &mut Vec<WindingVertex>,
    ) {
        for (start, parts) in &self.contours {
            let anchor = place(*start);
            let mut from = *start;
            for part in parts {
                let a = place(from);
                let b = place(part.to);
                // The chord fan: this segment's contribution to the winding number,
                // signed by the triangle's own orientation in the shader.
                out.push(WindingVertex::solid(anchor, clip));
                out.push(WindingVertex::solid(a, clip));
                out.push(WindingVertex::solid(b, clip));
                if let Some(control) = part.control {
                    // Loop and Blinn's control triangle. The texture coordinates are
                    // theirs: a quadratic is `u² − v = 0`, so the three control points
                    // take (0,0), (½,0) and (1,1) and the interior of the curve is
                    // where `u² < v`. Orientation does the rest — a control point
                    // outside the chord makes this triangle wind with the fan and add
                    // its bulge; inside, it winds against it and takes the bite out.
                    out.push(WindingVertex::curve(a, [0.0, 0.0], clip));
                    out.push(WindingVertex::curve(place(control), [0.5, 0.0], clip));
                    out.push(WindingVertex::curve(b, [1.0, 1.0], clip));
                }
                from = part.to;
            }
        }
    }
}

/// Appends triangles for already-flattened device-space polylines.
///
/// The stroke and oblique-rectangle lanes arrive here rather than at
/// [`QuadOutline::append_triangles`]: a stroke's outline is expanded to a polygon on
/// the CPU (§8.4.3's caps, joins and miters), and what comes out is straight edges. The
/// fan is the same, and so is the rule that gives it its sign — only the curve
/// triangles are missing, because there are no curves left to carry.
///
/// The scale-independence [`QuadOutline`] buys does not apply to this path, and saying
/// so is the point: a stroke still flattens per frame. What moves to the device is the
/// *rasterisation*, which is the half ADR 0015 measured as the cost.
pub(crate) fn append_polyline_triangles(
    polylines: &[crate::raster::Polyline],
    place: impl Fn(Point) -> [f32; 2],
    clip: [f32; 4],
    out: &mut Vec<WindingVertex>,
) {
    for polyline in polylines {
        let Some(start) = polyline.points.first() else {
            continue;
        };
        if polyline.points.len() < 3 {
            continue; // no area to enclose, as in `finish_contour`
        }
        let anchor = place(*start);
        // Closed by construction here: an expanded stroke is a closed polygon, and the
        // chord from the last point back to the first is what makes it one.
        let points = polyline.points.iter().chain(std::iter::once(start));
        let mut from = *start;
        for to in points.skip(1) {
            out.push(WindingVertex::solid(anchor, clip));
            out.push(WindingVertex::solid(place(from), clip));
            out.push(WindingVertex::solid(place(*to), clip));
            from = *to;
        }
    }
}

/// One vertex of the winding pass.
///
/// `uv` carries Loop and Blinn's implicit coordinates. A straight triangle uses
/// `(0, 1)` at all three corners, where `u² < v` holds everywhere — so the fragment
/// stage has one test and no branch, and a chord triangle is simply a curve triangle
/// whose test can never fail.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub(crate) struct WindingVertex {
    position: [f32; 2],
    uv: [f32; 2],
    clip: [f32; 4],
}

impl WindingVertex {
    /// Bytes one vertex occupies in the buffer the winding pass reads.
    pub(crate) const STRIDE: u64 = 32;

    fn solid(position: [f32; 2], clip: [f32; 4]) -> Self {
        Self {
            position,
            uv: [0.0, 1.0],
            clip,
        }
    }

    fn curve(position: [f32; 2], uv: [f32; 2], clip: [f32; 4]) -> Self {
        Self { position, uv, clip }
    }

    /// The vertex as the eight floats the pipeline's layout expects.
    pub(crate) fn floats(self) -> [f32; 8] {
        [
            self.position[0],
            self.position[1],
            self.uv[0],
            self.uv[1],
            self.clip[0],
            self.clip[1],
            self.clip[2],
            self.clip[3],
        ]
    }
}

/// Subdivides a cubic until the quadratic bound holds, appending what it converged to.
#[allow(clippy::arithmetic_side_effects)] // f32 geometry on coordinates the scene
// boundary bounded by MAX_COORDINATE, plus a depth MAX_SPLIT_DEPTH already caps
fn push_cubic(
    out: &mut Vec<QuadSegment>,
    p0: Point,
    c1: Point,
    c2: Point,
    p3: Point,
    tolerance: f32,
    depth: u8,
) {
    // The third difference, whose length bounds the distance from the cubic to its
    // quadratic approximation by `√3/36` — and which is exactly zero when the cubic is
    // a degree-elevated quadratic, so an already-quadratic curve converts exactly.
    let dx = p3.x - 3.0 * c2.x + 3.0 * c1.x - p0.x;
    let dy = p3.y - 3.0 * c2.y + 3.0 * c1.y - p0.y;
    if depth >= MAX_SPLIT_DEPTH || THIRD_DIFFERENCE_BOUND * dx.hypot(dy) <= tolerance {
        out.push(QuadSegment {
            control: Some(Point::new(
                0.75 * (c1.x + c2.x) - 0.25 * (p0.x + p3.x),
                0.75 * (c1.y + c2.y) - 0.25 * (p0.y + p3.y),
            )),
            to: p3,
        });
        return;
    }
    // de Casteljau at t = ½: exact in `f32`, since halving is exact.
    let mid = |a: Point, b: Point| Point::new(0.5 * (a.x + b.x), 0.5 * (a.y + b.y));
    let (ab, bc, cd) = (mid(p0, c1), mid(c1, c2), mid(c2, p3));
    let (abc, bcd) = (mid(ab, bc), mid(bc, cd));
    let apex = mid(abc, bcd);
    push_cubic(out, p0, ab, abc, apex, tolerance, depth + 1);
    push_cubic(out, apex, bcd, cd, p3, tolerance, depth + 1);
}

#[cfg(test)]
#[allow(clippy::expect_used)] // test-file policy as in `raster.rs`
mod tests {
    use super::{QuadOutline, WindingVertex};
    use quorra_scene::{Point, Segment};

    /// A cubic that *is* a quadratic converts to exactly one segment, exactly placed:
    /// the third difference is zero, so the bound holds at depth zero and the control
    /// point formula inverts degree elevation.
    #[test]
    fn a_degree_elevated_quadratic_converts_exactly() {
        // The quadratic (0,0) — (4,8) control — (8,0), elevated to a cubic.
        let q = Point::new(4.0, 8.0);
        let (p0, p3) = (Point::new(0.0, 0.0), Point::new(8.0, 0.0));
        let elevate = |end: Point| Point::new((end.x + 2.0 * q.x) / 3.0, (end.y + 2.0 * q.y) / 3.0);
        let outline = QuadOutline::from_segments(&[
            Segment::MoveTo(p0),
            Segment::CubicTo {
                c1: elevate(p0),
                c2: elevate(p3),
                to: p3,
            },
            Segment::Close,
        ]);
        // One curve, plus the closing chord back to the start.
        assert_eq!(outline.triangle_count(), 1 + 2, "one curve and one chord");
        let mut vertices = Vec::new();
        outline.append_triangles(|p| [p.x, p.y], [0.0, 0.0, 8.0, 8.0], &mut vertices);
        let control = vertices[4].floats();
        assert!(
            (control[0] - q.x).abs() < 1e-4 && (control[1] - q.y).abs() < 1e-4,
            "the control point comes back where it started: {control:?}"
        );
    }

    /// An open contour is closed before it is filled (§8.5.3.1), so the chord back to
    /// the start is part of the geometry rather than something a caller must add.
    #[test]
    fn an_open_contour_is_closed_by_a_chord() {
        let outline = QuadOutline::from_segments(&[
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(4.0, 0.0)),
            Segment::LineTo(Point::new(4.0, 4.0)),
        ]);
        assert_eq!(
            outline.triangle_count(),
            3,
            "two stated segments and the closing chord"
        );
    }

    /// A contour that cannot enclose area emits nothing: two triangles that cancel are
    /// not worth the vertices, and a fill of them is transparent either way.
    #[test]
    fn a_degenerate_contour_emits_no_triangles() {
        let outline = QuadOutline::from_segments(&[
            Segment::MoveTo(Point::new(1.0, 1.0)),
            Segment::LineTo(Point::new(9.0, 9.0)),
            Segment::Close,
        ]);
        assert!(outline.is_empty());
        assert_eq!(outline.triangle_count(), 0);
    }

    /// A straight triangle's implicit coordinates keep every fragment: `u² < v` holds
    /// at (0, 1), which is what lets one fragment shader serve both triangle kinds.
    #[test]
    fn a_chord_triangle_is_never_discarded() {
        let outline = QuadOutline::from_segments(&[
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(4.0, 0.0)),
            Segment::LineTo(Point::new(4.0, 4.0)),
            Segment::Close,
        ]);
        let mut vertices = Vec::new();
        outline.append_triangles(|p| [p.x, p.y], [0.0, 0.0, 4.0, 4.0], &mut vertices);
        assert_eq!(vertices.len(), 9, "three chords, three vertices each");
        for vertex in &vertices {
            let floats = vertex.floats();
            assert!(
                floats[2] * floats[2] < floats[3],
                "a chord vertex is inside the implicit curve: {floats:?}"
            );
        }
    }

    /// The vertex is what the pipeline's layout says it is. Stated as a test because
    /// the two live in different files and nothing else would notice them diverging.
    #[test]
    fn the_vertex_stride_matches_its_floats() {
        assert_eq!(
            usize::try_from(WindingVertex::STRIDE).expect("a stride fits a usize"),
            size_of::<[f32; 8]>(),
            "eight floats, and the pipeline reads them as one array"
        );
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss
)]
mod conversion_error {
    use super::QuadOutline;
    use quorra_scene::{Point, Segment};

    /// How far the converted quadratics sit from the cubic they replace, sampled.
    #[test]
    fn the_conversion_stays_inside_its_stated_bound() {
        let (p0, c1, c2, p3) = (
            Point::new(8.0, 8.0),
            Point::new(40.0, 2.0),
            Point::new(44.0, 30.0),
            Point::new(24.0, 40.0),
        );
        let outline = QuadOutline::from_segments(&[
            Segment::MoveTo(p0),
            Segment::CubicTo { c1, c2, to: p3 },
            Segment::Close,
        ]);
        let mut vertices = Vec::new();
        outline.append_triangles(|p| [p.x, p.y], [0.0, 0.0, 64.0, 64.0], &mut vertices);
        // Every curve triangle's three vertices, in order: (from, control, to).
        let mut quads = Vec::new();
        for triangle in vertices.chunks_exact(3) {
            let uv = triangle[1].floats();
            if (uv[2] - 0.5).abs() < 1e-6 && uv[3].abs() < 1e-6 {
                let f = |v: &super::WindingVertex| {
                    let x = v.floats();
                    Point::new(x[0], x[1])
                };
                quads.push((f(&triangle[0]), f(&triangle[1]), f(&triangle[2])));
            }
        }
        assert!(!quads.is_empty(), "the cubic converted to quadratics");
        eprintln!("{} quadratics", quads.len());

        let cubic_at = |t: f32| {
            let u = 1.0 - t;
            Point::new(
                u * u * u * p0.x
                    + 3.0 * u * u * t * c1.x
                    + 3.0 * u * t * t * c2.x
                    + t * t * t * p3.x,
                u * u * u * p0.y
                    + 3.0 * u * u * t * c1.y
                    + 3.0 * u * t * t * c2.y
                    + t * t * t * p3.y,
            )
        };
        let mut worst = 0.0_f32;
        for i in 0..=200 {
            let point = cubic_at(i as f32 / 200.0);
            let mut nearest = f32::INFINITY;
            for (a, b, c) in &quads {
                for j in 0..=100 {
                    let s = j as f32 / 100.0;
                    let r = 1.0 - s;
                    let q = Point::new(
                        r * r * a.x + 2.0 * r * s * b.x + s * s * c.x,
                        r * r * a.y + 2.0 * r * s * b.y + s * s * c.y,
                    );
                    nearest = nearest.min((q.x - point.x).hypot(q.y - point.y));
                }
            }
            worst = worst.max(nearest);
        }
        eprintln!("worst deviation {worst} units");
        assert!(worst < 0.01, "conversion drifted by {worst} units");
    }
}

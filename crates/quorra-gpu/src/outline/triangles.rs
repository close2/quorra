//! The triangles a filled outline becomes, and the vertex the winding pass reads.
//!
//! ISO 32000-2 §8.5.3.3's winding number, expressed as geometry: the chord fan sums to
//! it, and Loop and Blinn's control triangle adds or subtracts the bulge between a chord
//! and its curve purely by its own orientation. Everything here runs **per placement,
//! per frame** — its input is already-converted geometry plus a map into device space —
//! which is what separates it from its parent, where the conversion runs once per outline
//! — on the first frame that reads it (ADR 0075) — and never again.
//!
//! [`WindingVertex`] is the seam with the device: `pipeline/spec.rs` states its attribute
//! layout, `pane.rs` and `winding/sheet.rs` count and write it, and its
//! [`STRIDE`](WindingVertex::STRIDE) is what a tile's vertex range is priced in.

use quorra_scene::Point;

use super::QuadOutline;

impl QuadOutline {
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

    /// Floats one vertex occupies, which is [`WindingVertex::STRIDE`] in the unit the
    /// sheet's buffer is built in — a tile's vertex range is counted in vertices, and
    /// the two must not drift apart.
    pub(crate) const FLOATS: usize = 8;

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

#[cfg(test)]
#[allow(clippy::expect_used)] // test-file policy as in `raster.rs`
mod tests {
    use super::{QuadOutline, WindingVertex};
    use quorra_scene::{Point, Segment};

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

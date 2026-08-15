//! The device box of a placement is its neighbour's, translated.
//!
//! Bounding an outline was the largest single term in a dense page's encode: callgrind
//! on the dense-text archetype put the direct computation at **28.1 % of the whole
//! encode** — 4 320 fills each transforming all 37 control points of a twelve-cubic
//! letterform and min/maxing them into a box, on a page holding 818 distinct outlines
//! under one linear part. The other 3 502 boxes are those 818 boxes moved.
//!
//! So this memo holds the box under the *linear* part of the device transform, keyed by
//! `(outline, linear bits)`, and each placement adds its own translation back. It lives
//! for one frame and is invisible above [`Encoder`](super::Encoder): the bounds it
//! returns are the bounds the direct computation returns, **bit for bit**, which is why
//! no counter, no lane choice and no pixel moves.
//!
//! **The benchmark, and it is the whole justification** (`doc/adr/0045`, callgrind on
//! the dense-text archetype — 4 320 commands, 818 outlines, 1191×1684, `Coverage::Cpu`,
//! quantum 16, two warm-up encodes then one steady one):
//!
//! | | Ir a steady encode |
//! |---|---:|
//! | direct, every placement | 18 434 963 |
//! | this memo | **14 524 976** |
//!
//! **−3 909 987, which is 21.2 % of the encode and 27.6 % of its recording phase**, with
//! `tests/archetypes.rs`'s counter row unchanged to the digit. What it costs in the
//! other direction is one hash and one probe per placement — 1.26 M for the 4 320 of
//! them, against the 5.17 M the transforms cost — and this module, which is the
//! readability half of the trade CLAUDE.md asks to be written down.
//!
//! **And on a second page shape, because `keyhash`'s own lesson is that a change measured
//! at one table size is measured once.** The artwork archetype — 900 commands over 300
//! outlines of 24 cubics, 405 of them strokes, 600 under a curve clip — reads
//! 621 599 548 → 620 321 847, which is **−0.21 %**: its encode is 34× more instructions a
//! command than dense text's and effectively all of it is the 600 residue tiles being
//! rasterised, so bounding is 0.2 % of it either way. The memo is worth a fifth of the
//! encode on the shape the brief's §0 is about and is a wash on the shape it is not.
//! Neither page loses, which is the property a per-placement probe had to earn.
//!
//! # Why "bit for bit" is a theorem and not a hope
//!
//! [`super::device_space::apply`] evaluates `a·x + c·y + e` as two multiplies and two additions, so
//! each transformed coordinate is `fl(sᵢ + e)` where `sᵢ = fl(fl(a·xᵢ) + fl(c·yᵢ))` is
//! the linear part alone. IEEE 754 addition is *correctly rounded*, and correct rounding
//! is monotone: `sᵢ ≥ s_min` implies `fl(sᵢ + e) ≥ fl(s_min + e)`. So the minimum of the
//! translated coordinates is attained at the same point as the minimum of the linear
//! ones and equals `fl(s_min + e)` exactly — which is what this memo stores and adds.
//! The same argument, mirrored, holds for the maximum.
//!
//! The argument needs the coordinates to be free of NaN, and they are by construction
//! rather than by luck: a scene's coordinates and transform coefficients are bounded by
//! [`MAX_COORDINATE`] (10⁹) and a viewport whose transform is not finite is refused
//! before any of this runs, so a composed coefficient is at most ~2 × 10¹⁸ and a
//! transformed coordinate at most ~2 × 10²⁷ — finite in `f32`, whose range ends at
//! 3.4 × 10³⁸. `a_memoised_box_is_the_direct_box` in this module's tests is the check that the theorem
//! holds for the arithmetic as written, over shapes and transforms rather than over one.
//!
//! [`MAX_COORDINATE`]: quorra_scene::MAX_COORDINATE

use quorra_scene::{OutlineId, Point, Segment};

use crate::keyhash::FastMap;
use crate::raster::DeviceTransform;

/// What a hull box is the same for: one outline under one linear map.
///
/// The linear part as bits rather than floats, because this is a hash key and `f32` is
/// not `Eq` — the identity `GlyphKey::linear` and [`Census`] key on as well, so a page
/// that collapses to few atlas keys collapses to few boxes for the same reason.
///
/// The outline's *identity* rather than its points, which is sound because the encoder
/// holds the [`ResourceStore`] immutably for the whole walk: no upload and no release
/// can happen between two placements of one frame, so an id names the same segments
/// every time this memo is asked.
///
/// [`Census`]: crate::census::Census
/// [`ResourceStore`]: crate::resources::ResourceStore
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct HullKey {
    outline: u32,
    linear: [u32; 4],
}

/// One frame's memo of control-hull boxes under the linear part of a device transform.
///
/// Grown rather than reserved: the count of distinct `(outline, linear)` pairs is not
/// known before the walk, the census that would bound it is taken only for the GPU lane
/// (ADR 0029), and the previous frame's count is exactly the guess ADR 0029 refuses. The
/// growth is bounded by the scene's own command count — one entry per placement at
/// worst, ~48 bytes against the 96 the frame budget has already charged that command for
/// its two instance streams — so this adds no allocation the budget check has not
/// already seen, in the same way `Encoder::atlas_keys` does not.
#[derive(Debug, Default)]
pub(crate) struct HullMemo {
    boxes: FastMap<HullKey, Option<[f32; 4]>>,
}

impl HullMemo {
    /// The device bounding box of `segments`' control hull under `t` — min x, min y,
    /// max x, max y — or `None` when the outline has no points at all.
    ///
    /// By the convex-hull property of Béziers this bounds the curve itself, which is the
    /// property both call sites rely on: a cull that used a bound the curve could leave
    /// would drop a command that marks the page.
    pub(crate) fn bounds(
        &mut self,
        outline: OutlineId,
        segments: &[Segment],
        t: &DeviceTransform,
    ) -> Option<(f32, f32, f32, f32)> {
        let key = HullKey {
            outline: outline.0,
            linear: [t.a.to_bits(), t.b.to_bits(), t.c.to_bits(), t.d.to_bits()],
        };
        let [x0, y0, x1, y1] = (*self
            .boxes
            .entry(key)
            .or_insert_with(|| linear_hull_bounds(segments, t)))?;
        Some((x0 + t.e, y0 + t.f, x1 + t.e, y1 + t.f))
    }
}

/// The control hull's box under the linear part of `t` alone.
///
/// The translation is added back per placement by [`HullMemo::bounds`], and the module
/// comment is why doing it in that order changes not one bit of the answer.
fn linear_hull_bounds(segments: &[Segment], t: &DeviceTransform) -> Option<[f32; 4]> {
    let mut bounds: Option<[f32; 4]> = None;
    let mut extend = |p: Point| {
        let (x, y) = (t.a * p.x + t.c * p.y, t.b * p.x + t.d * p.y);
        bounds = Some(match bounds {
            None => [x, y, x, y],
            Some(b) => [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)],
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
    bounds
}

#[cfg(test)]
mod tests {
    use super::{HullMemo, linear_hull_bounds};
    use crate::encode::device_space::apply;
    use crate::raster::DeviceTransform;
    use quorra_scene::{OutlineId, Point, Segment};

    /// The computation this memo replaces, written out once so the test compares against
    /// the arithmetic rather than against another memo.
    fn direct(segments: &[Segment], t: &DeviceTransform) -> Option<(f32, f32, f32, f32)> {
        let mut bounds: Option<(f32, f32, f32, f32)> = None;
        let mut extend = |p: Point| {
            let q = apply(t, p);
            bounds = Some(match bounds {
                None => (q.x, q.y, q.x, q.y),
                Some((x0, y0, x1, y1)) => (x0.min(q.x), y0.min(q.y), x1.max(q.x), y1.max(q.y)),
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
        bounds
    }

    /// A handful of shapes whose points are *not* round numbers, because a box that
    /// agrees on halves and quarters says nothing about the rounding this memo moves.
    fn shapes() -> Vec<Vec<Segment>> {
        let p = |x: f32, y: f32| Point::new(x, y);
        vec![
            vec![],
            vec![Segment::MoveTo(p(0.318_407_3, -0.577_931_4))],
            vec![
                Segment::MoveTo(p(-3.142_881_3, 2.719_913_4)),
                Segment::LineTo(p(11.181_772, -0.694_411_6)),
                Segment::LineTo(p(-7.390_337, 9.871_223)),
                Segment::Close,
            ],
            vec![
                Segment::MoveTo(p(1.415_774_2, 1.733_611_4)),
                Segment::CubicTo {
                    c1: p(2.237_629, -1.261_482),
                    c2: p(-5.658_415, 0.302_591_2),
                    to: p(8.661_815, -4.670_763),
                },
                Segment::CubicTo {
                    c1: p(-0.001_234_5, 12_345.678),
                    c2: p(6.284_746_5, -2.304_146),
                    to: p(1.415_774_2, 1.733_611_4),
                },
                Segment::Close,
            ],
        ]
    }

    /// The module's theorem, checked as equality of bits rather than of pictures: a box
    /// built from the linear part and translated afterwards is the box built from the
    /// whole affine, for every shape above under every transform below.
    ///
    /// Bit equality is the right assertion and not a strict one for its own sake. These
    /// bounds decide a cull, a tile's `floor`/`ceil` extent and which lane a fill takes,
    /// and each of those is a step function of the value — so "close enough" is exactly
    /// the tolerance under which a page changes without a test seeing it.
    #[test]
    fn a_memoised_box_is_the_direct_box() {
        let transforms = [
            DeviceTransform {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e: 0.0,
                f: 0.0,
            },
            DeviceTransform {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e: 0.317_9,
                f: -1_024.75,
            },
            DeviceTransform {
                a: 2.831_2,
                b: 0.0,
                c: 0.0,
                d: -2.831_2,
                e: 917.331,
                f: 0.007,
            },
            // A rotation and a shear: the terms this memo drops from the inner loop are
            // exactly the ones a non-diagonal linear part makes expensive.
            DeviceTransform {
                a: 0.866_025_4,
                b: 0.5,
                c: -0.5,
                d: 0.866_025_4,
                e: -33.7,
                f: 12.9,
            },
            DeviceTransform {
                a: 1.0,
                b: 0.311,
                c: 0.717,
                d: 1.0,
                e: 1e6,
                f: -1e6,
            },
            // Far from the origin, where a translation added last and a translation
            // added first are most likely to round to different floats — and do not.
            DeviceTransform {
                a: 1e3,
                b: 0.0,
                c: 0.0,
                d: 1e3,
                e: 1e9,
                f: -1e9,
            },
        ];
        for (index, segments) in shapes().iter().enumerate() {
            for t in &transforms {
                let mut memo = HullMemo::default();
                // Twice, so the answer from the populated memo is checked as well as the
                // answer that filled it.
                for _ in 0..2 {
                    let outline = OutlineId(u32::try_from(index).unwrap());
                    let memoised = memo.bounds(outline, segments, t);
                    let expected = direct(segments, t);
                    match (memoised, expected) {
                        (None, None) => {}
                        (Some(m), Some(e)) => assert_eq!(
                            [m.0.to_bits(), m.1.to_bits(), m.2.to_bits(), m.3.to_bits()],
                            [e.0.to_bits(), e.1.to_bits(), e.2.to_bits(), e.3.to_bits()],
                            "shape {index} under {t:?}",
                        ),
                        (m, e) => panic!("shape {index} under {t:?}: {m:?} against {e:?}"),
                    }
                }
            }
        }
    }

    /// Two outlines that share a linear part get their own boxes, and one outline under
    /// two linear parts gets two — the key is both halves, and a memo keyed on either
    /// alone would draw one letterform's box around another's.
    #[test]
    fn the_key_is_the_outline_and_the_linear_part_together() {
        let square = vec![
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(4.0, 3.0)),
            Segment::Close,
        ];
        let wide = vec![
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(40.0, 3.0)),
            Segment::Close,
        ];
        let one = DeviceTransform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 5.0,
            f: 5.0,
        };
        let two = DeviceTransform {
            a: 3.0,
            b: 0.0,
            c: 0.0,
            d: 3.0,
            e: 5.0,
            f: 5.0,
        };
        let mut memo = HullMemo::default();
        assert_eq!(
            memo.bounds(OutlineId(0), &square, &one),
            Some((5.0, 5.0, 9.0, 8.0))
        );
        assert_eq!(
            memo.bounds(OutlineId(1), &wide, &one),
            Some((5.0, 5.0, 45.0, 8.0))
        );
        assert_eq!(
            memo.bounds(OutlineId(0), &square, &two),
            Some((5.0, 5.0, 17.0, 14.0))
        );
        assert_eq!(
            memo.bounds(OutlineId(0), &square, &one),
            Some((5.0, 5.0, 9.0, 8.0))
        );
    }

    /// An outline with no points has no box, and asking twice does not turn that into a
    /// box: the `None` is memoised like any other answer.
    #[test]
    fn an_outline_with_no_points_has_no_box() {
        let t = DeviceTransform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 7.0,
            f: 7.0,
        };
        assert_eq!(linear_hull_bounds(&[], &t), None);
        let mut memo = HullMemo::default();
        assert_eq!(memo.bounds(OutlineId(0), &[], &t), None);
        assert_eq!(memo.bounds(OutlineId(0), &[], &t), None);
    }
}

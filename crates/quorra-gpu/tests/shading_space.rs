//! A shading pattern is anchored to the page, not to the mark it paints
//! (ISO 32000-2 §8.7.2 and §8.7.4.1; integration note 9).
//!
//! # The clause, and where it actually is
//!
//! The rule is stated in **§8.7.2**, about the pattern matrix every Type 2 pattern
//! dictionary carries (Table 75, §8.7.4.1):
//!
//! > Every pattern has a pattern matrix , a transformation matrix that maps the pattern's
//! > internal coordinate system to the default coordinate system of the pattern's parent
//! > content stream (the content stream in which the pattern is defined as a resource).
//!
//! and, two sentences later, the consequence this file is about:
//!
//! > Changes to the page's transformation matrix that occur within the page's content
//! > stream, such as rotation and scaling, have no effect on the pattern; it maintains its
//! > original relationship to the page no matter where on the page it is used.
//!
//! §8.7.4.1 says the same thing from the painting operator's side, for `f`, `S` and `Tj`
//! alike — which is why one file gates a fill, a stroke and a glyph-sized mark together:
//!
//! > By setting a shading pattern as the current colour in the graphics state, a PDF
//! > content stream may use it with painting operators such as f (fill), S (stroke), Tj
//! > (show text), or Do (paint external object) with an image mask to paint a path,
//! > character glyph, or mask with a smooth colour transition. When a shading is used in
//! > this way, the geometry of the gradient fill is independent of that of the object
//! > being painted.
//!
//! §8.7.4.3 is where the *name* for that space is given — its NOTE 2 defines "target
//! coordinate space" and points at §8.7.2 for what it is — so a citation of §8.7.4.3
//! alone lands one subclause away from the sentence that decides anything.
//!
//! # What is under test here
//!
//! [`Paint::Shading`] carries its geometry in the shading's own space and a `transform`
//! that maps it into the scene; the shaded *command's* own transform is deliberately not
//! composed into it (`encode/rare.rs`'s `rare_paint`). The failure this gate exists to
//! catch is one matrix too many: the same paint on the same device pixels reading the ramp
//! at a different place because the mark was placed by a translation instead of by its own
//! coordinates.
//!
//! So every test below draws **the same device pixels three ways** — by the outline's own
//! coordinates, by a command translation, and by a translation composed with a scale — and
//! requires the three frames to be identical to the byte. That is stronger than a
//! tolerance and it is exactly the clause: the placement changed, the pattern did not.
//!
//! # Which lane each fixture means (`doc/HANDOVER.md`'s lane trap)
//!
//! The three marks take three different paths to the same paint, and each test says which
//! by asserting counters rather than by assuming:
//!
//! - a rect-hinted fill takes `push_rare_rect`'s analytic coverage — **no** scratch tile;
//! - a triangle fill and every stroke take `push_rare_coverage` — one tile each;
//! - a glyph-sized outline placed more than once takes the **atlas** when it is filled
//!   solid, and does **not** when the paint is a shading. That is the answer to the
//!   caller's #102 for this tree and it is asserted in both directions, so the fixture
//!   cannot quietly stop meaning "glyph lane".

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Counters, Device, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, LineCap, LineJoin, OutlineId, Paint, Point,
    RampId, Rect, Scene, SceneBuilder, Segment, ShadingKind, Stop, Stroke,
};

mod common;

use common::headless::device;
use common::probe::pixel;
use common::scene::rect_outline;

/// The target every frame in this file is drawn into. 64 × 4 = 256 bytes a row, which is
/// exactly the buffer-copy alignment, and wide enough to hold two marks that must not
/// touch.
const SIZE: u32 = 64;

/// The axial shading every test paints with: black at device x = 0, white at x = [`SIZE`],
/// stated in the shading's own space under the identity transform so that "the shading's
/// own space" and "the page" coincide and a misplaced ramp is a *visible* offset rather
/// than a scale.
///
/// `extend` is true at both ends (§8.7.4.5.3's `Extend`) so that no part of the target is
/// unpainted for a reason other than the mark's own coverage.
fn page_wide_ramp(ramp: RampId) -> Paint {
    Paint::Shading {
        ramp,
        kind: ShadingKind::Axial {
            start: Point::new(0.0, 0.0),
            end: Point::new(SIZE as f32, 0.0),
            extend: (true, true),
        },
        transform: Affine::IDENTITY,
    }
}

/// Black at 0, white at 1 — two stops, so §7.10.3's type 2 with an exponent of 1 gives the
/// colour at `t` as `t` in every channel, and the expectation below is arithmetic.
fn grey_ramp(device: &mut Device) -> RampId {
    device
        .upload_ramp(&[
            Stop {
                offset: 0.0,
                color: Color::new(0.0, 0.0, 0.0, 1.0),
            },
            Stop {
                offset: 1.0,
                color: Color::new(1.0, 1.0, 1.0, 1.0),
            },
        ])
        .expect("two ascending stops spanning 0..=1")
}

/// The grey byte [`page_wide_ramp`] puts at device column `x`, derived from the clause.
///
/// §8.7.4.5.3 parametrises an axial shading by the projection of the point onto the axis;
/// the point is the pixel's centre (§10.7.4: "The position of the centre of such a pixel -
/// in other words, the point whose coordinate values have fractional parts of one-half"),
/// so `t = (x + 0.5) / SIZE`. §7.10.3's type 2 with N = 1 makes the colour `t`, and ADR
/// 0006 stores it as `round(t · 255)`.
///
/// Callers compare within ±1: the ramp is pre-sampled to 4 096 texels at upload (ADR
/// 0011), which displaces `t` by at most half a texel — under a tenth of an 8-bit level —
/// and the store rounds once more.
fn clause_grey_at(x: u32) -> u8 {
    let t = (x as f32 + 0.5) / SIZE as f32;
    (t * 255.0).round() as u8
}

/// Draw `scene` and hand back its pixels together with the counters that say which lane
/// drew it.
fn render(device: &mut Device, scene: &Scene) -> (Vec<u8>, Counters) {
    let frame = device
        .render(
            scene,
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the frame is inside every budget");
    let counters = frame.counters();
    (frame.into_raster().unwrap().into_pixels(), counters)
}

/// The three ways this file places one mark on the same device pixels, as
/// (outline offset, command transform) pairs.
///
/// A mark whose outline is written at `offset` and drawn under the paired transform lands
/// in the same place for all three, and the three transforms are as different as an affine
/// gets while still doing so: identity, a translation, and a translation composed with a
/// scale. Under §8.7.2 the paint may not notice.
const PLACEMENTS: [(f32, f32); 3] = [(32.0, 1.0), (0.0, 1.0), (0.0, 2.0)];

fn placement_transform(index: usize) -> Affine {
    let (offset, scale) = PLACEMENTS[index];
    // The outline is written at `offset` in its own coordinates and is `scale` times
    // smaller than the device mark, so the transform is whatever carries that to device
    // (32, 32): the identity when the outline already sits there, a translation when it
    // sits at the origin, and a scale followed by a translation for the third.
    Affine::scale(scale, scale).then(Affine::translate(32.0 - offset, 32.0 - offset))
}

/// The square each placement covers in device space: 8 × 8 at (32, 32).
fn placed_square(index: usize) -> Rect {
    let (offset, scale) = PLACEMENTS[index];
    Rect::new(
        Point::new(offset, offset),
        Point::new(offset + 8.0 / scale, offset + 8.0 / scale),
    )
}

/// A closed triangle inscribed in `rect`, which `axis_aligned_rect` refuses — so a fill of
/// it takes the rasterised-coverage path rather than the analytic rectangle one.
fn triangle_in(rect: Rect) -> Vec<Segment> {
    vec![
        Segment::MoveTo(rect.min),
        Segment::LineTo(Point::new(rect.max.x, rect.min.y)),
        Segment::LineTo(Point::new(rect.min.x, rect.max.y)),
        Segment::Close,
    ]
}

/// A short open diagonal inside `rect`, for the stroke fixtures.
fn diagonal_in(rect: Rect) -> Vec<Segment> {
    vec![Segment::MoveTo(rect.min), Segment::LineTo(rect.max)]
}

fn fill_scene(outline: OutlineId, transform: Affine, paint: Paint) -> Scene {
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            transform,
            FillRule::NonZero,
            paint,
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("a valid shaded fill");
    builder.finish()
}

fn stroke_scene(outline: OutlineId, transform: Affine, width: f32, paint: Paint) -> Scene {
    let mut builder = SceneBuilder::new();
    builder
        .stroke(
            outline,
            transform,
            Stroke {
                width,
                cap: LineCap::Butt,
                join: LineJoin::Miter,
                miter_limit: 10.0,
            },
            paint,
            None,
            BlendMode::Normal,
            None,
        )
        .expect("a valid shaded stroke");
    builder.finish()
}

/// Draw the same device mark three times, once per entry of [`PLACEMENTS`], checking on
/// the way that each frame took the lane the caller named.
///
/// `scene_at` is handed the placement's index and the device it may upload through.
fn three_placements(
    device: &mut Device,
    expected_tiles: u32,
    mut scene_at: impl FnMut(&mut Device, usize) -> Scene,
) -> [Vec<u8>; 3] {
    let frames: Vec<Vec<u8>> = (0..PLACEMENTS.len())
        .map(|index| {
            let scene = scene_at(device, index);
            let (pixels, counters) = render(device, &scene);
            assert_eq!(
                counters.tiles, expected_tiles,
                "placement {index} reached the sheet {} times where {expected_tiles} was \
                 named — this fixture no longer means the lane it says it does",
                counters.tiles
            );
            pixels
        })
        .collect();
    frames.try_into().expect("one frame per placement")
}

/// Assert that three frames of the same mark, placed by three different matrices, are the
/// same bytes — and that at least one of them actually put ink down, so the equality is
/// not three empty targets agreeing.
fn assert_the_paint_did_not_follow_the_mark(frames: &[Vec<u8>; 3], what: &str) {
    let inked = frames[0]
        .iter()
        .skip(3)
        .step_by(4)
        .filter(|a| **a > 0)
        .count();
    assert!(
        inked > 32,
        "{what}: the fixture drew almost nothing ({inked} covered pixels), so an equality \
         between placements would prove nothing"
    );
    assert_eq!(
        frames[0], frames[1],
        "{what}: a command translation moved the ramp — §8.7.2's pattern matrix is \
         composed with the mark's transform somewhere it should not be"
    );
    assert_eq!(
        frames[0], frames[2],
        "{what}: a command scale moved the ramp — §8.7.2's pattern matrix is composed \
         with the mark's transform somewhere it should not be"
    );
}

/// A rect-hinted fill: the analytic branch of the rare lane, and the one that places its
/// quad from the shape's device rectangle rather than from a tile.
#[test]
fn a_rect_fill_reads_the_ramp_at_the_page_position_however_it_was_placed() {
    let mut device = device();
    let ramp = grey_ramp(&mut device);
    // Nought tiles: a rect-hinted shaded fill takes `push_rare_rect`'s analytic coverage.
    let frames = three_placements(&mut device, 0, |device, index| {
        let outline = device
            .upload_outline(&rect_outline(placed_square(index)))
            .expect("upload");
        fill_scene(outline, placement_transform(index), page_wide_ramp(ramp))
    });
    assert_the_paint_did_not_follow_the_mark(&frames, "a rect-hinted shaded fill");

    // And the ramp is where §8.7.2 puts it, not merely in the same wrong place three
    // times: the mark spans device columns 32..40 and each column carries its own t.
    for x in 32..40 {
        let expected = clause_grey_at(x);
        let actual = pixel(&frames[0], SIZE, x, 36);
        assert!(
            i32::from(actual[0]).abs_diff(i32::from(expected)) <= 1,
            "column {x}: the ramp reads {} where §8.7.4.5.3 with t = (x + 0.5)/{SIZE} \
             gives {expected}",
            actual[0]
        );
        assert_eq!(actual[3], 255, "column {x}: the square is opaque");
    }
}

/// A triangle fill: the rasterised branch of the rare lane, one coverage tile.
#[test]
fn a_rasterised_fill_reads_the_ramp_at_the_page_position_however_it_was_placed() {
    let mut device = device();
    let ramp = grey_ramp(&mut device);
    // One tile: a triangle is not a rectangle, so its coverage has to be rasterised.
    let frames = three_placements(&mut device, 1, |device, index| {
        let outline = device
            .upload_outline(&triangle_in(placed_square(index)))
            .expect("upload");
        fill_scene(outline, placement_transform(index), page_wide_ramp(ramp))
    });
    assert_the_paint_did_not_follow_the_mark(&frames, "a rasterised shaded fill");
}

/// Their #968: a gradient on a **stroke**. The stroke's expansion happens in device space
/// under the command's transform, so this is the lane where a paint that followed the mark
/// would follow it through two matrices rather than one.
#[test]
fn a_stroke_reads_the_ramp_at_the_page_position_however_it_was_placed() {
    let mut device = device();
    let ramp = grey_ramp(&mut device);
    // One tile: a shaded stroke reaches the sheet through its expansion, always.
    let frames = three_placements(&mut device, 1, |device, index| {
        let outline = device
            .upload_outline(&diagonal_in(placed_square(index)))
            .expect("upload");
        // The width is device-space and already resolved upstream (§4.5 of the brief), so
        // it does **not** change with the placement's scale: the same device band under
        // all three, which is what makes the three frames comparable at all.
        stroke_scene(
            outline,
            placement_transform(index),
            5.0,
            page_wide_ramp(ramp),
        )
    });
    assert_the_paint_did_not_follow_the_mark(&frames, "a shaded stroke");
}

/// The glyph-sized outline this file's #102 fixtures place: a small closed triangle, the
/// size a glyph is at reading size, written at the origin so a placement is a translation.
fn glyph_outline() -> Vec<Segment> {
    vec![
        Segment::MoveTo(Point::new(0.0, 0.0)),
        Segment::LineTo(Point::new(7.0, 0.0)),
        Segment::LineTo(Point::new(3.5, 9.0)),
        Segment::Close,
    ]
}

/// Where the repeated glyph is placed. More than one placement of one outline under one
/// linear part is what makes the census say the shape is worth caching (ADR 0029); a
/// single placement never reaches the atlas whatever its size, so a one-mark fixture
/// asserting `atlas_distinct_keys == 0` would prove nothing at all.
const GLYPH_RUN: [f32; 4] = [8.0, 20.0, 32.0, 44.0];

fn glyph_run_scene(outline: OutlineId, paint: Paint, extra: Affine) -> Scene {
    let mut builder = SceneBuilder::new();
    for x in GLYPH_RUN {
        builder
            .fill(
                outline,
                extra.then(Affine::translate(x, 28.0)),
                FillRule::NonZero,
                paint,
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .expect("a valid glyph-sized fill");
    }
    builder.finish()
}

/// Their #102, and the lane trap in one test: a run of one repeated outline is a
/// **glyph-lane** run when it is filled solid, and stops being one the moment the paint is
/// a shading. The control is the first half — without it, the second half's
/// `atlas_distinct_keys == 0` would be satisfied by an outline the atlas never wanted.
#[test]
fn a_shading_takes_a_glyph_run_off_the_atlas_lane() {
    let mut device = device();
    let ramp = grey_ramp(&mut device);
    let outline = device.upload_outline(&glyph_outline()).expect("upload");

    let solid = glyph_run_scene(
        outline,
        Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0)),
        Affine::IDENTITY,
    );
    let (_, solid_counters) = render(&mut device, &solid);
    assert_eq!(
        solid_counters.atlas_distinct_keys, 1,
        "the control: four placements of one outline are one atlas key, so this outline \
         at this size genuinely is a glyph-lane mark"
    );

    let shaded = glyph_run_scene(outline, page_wide_ramp(ramp), Affine::IDENTITY);
    let (_, shaded_counters) = render(&mut device, &shaded);
    assert_eq!(
        shaded_counters.atlas_distinct_keys, 0,
        "a shaded fill resolves its paint before any cache is consulted \
         (`encode/fill.rs`), so a gradient-filled glyph is not an atlas mark here"
    );
    assert_eq!(
        shaded_counters.tiles,
        GLYPH_RUN.len() as u32,
        "each shaded glyph rasterises its own coverage tile instead"
    );
}

/// Their #102's substance: four copies of one glyph, one shading, and the ramp read at
/// each glyph's own place on the page — the sweep crossing the run rather than restarting
/// inside every mark.
#[test]
fn a_shaded_glyph_run_sweeps_across_the_page_not_within_each_glyph() {
    let mut device = device();
    let ramp = grey_ramp(&mut device);
    let outline = device.upload_outline(&glyph_outline()).expect("upload");
    let (pixels, _) = render(
        &mut device,
        &glyph_run_scene(outline, page_wide_ramp(ramp), Affine::IDENTITY),
    );

    // Each glyph's baseline row, one pixel above its widest part, is solidly inside the
    // triangle; the byte there is the ramp at that column and nothing else.
    let mut seen = Vec::new();
    for x in GLYPH_RUN {
        let column = x as u32 + 3;
        let probe = pixel(&pixels, SIZE, column, 29);
        assert_eq!(probe[3], 255, "column {column} is inside its glyph");
        let expected = clause_grey_at(column);
        assert!(
            i32::from(probe[0]).abs_diff(i32::from(expected)) <= 1,
            "glyph at x = {x}: the ramp reads {} where §8.7.2 anchored to the page gives \
             {expected}",
            probe[0]
        );
        seen.push(probe[0]);
    }
    // Stated separately because the per-glyph check above would also pass if every glyph
    // were painted with one flat colour that happened to be right at one column.
    assert!(
        seen.windows(2).all(|pair| pair[1] > pair[0]),
        "the run must brighten left to right across the page: {seen:?}"
    );
}

/// The same glyph run under a command transform that scales and moves every mark: the
/// marks change place, the pattern does not. §8.7.2's second sentence, drawn.
#[test]
fn a_command_transform_on_a_glyph_run_moves_the_marks_and_not_the_ramp() {
    let mut device = device();
    let ramp = grey_ramp(&mut device);
    let outline = device.upload_outline(&glyph_outline()).expect("upload");
    let paint = page_wide_ramp(ramp);

    // Two runs whose *device* geometry is identical, reached by two different matrices:
    // once by the outline itself under a translation, once by a half-size outline under a
    // scale of two. Every coordinate involved halves and doubles exactly in binary
    // floating point, so the composed device transforms agree to the bit and any
    // difference in the frames is the paint's, not the flattener's.
    let plain = glyph_run_scene(outline, paint, Affine::IDENTITY);
    let half = device
        .upload_outline(
            &glyph_outline()
                .iter()
                .map(|segment| match *segment {
                    Segment::MoveTo(p) => Segment::MoveTo(Point::new(p.x / 2.0, p.y / 2.0)),
                    Segment::LineTo(p) => Segment::LineTo(Point::new(p.x / 2.0, p.y / 2.0)),
                    other => other,
                })
                .collect::<Vec<_>>(),
        )
        .expect("upload");
    let doubled = glyph_run_scene(half, paint, Affine::scale(2.0, 2.0));
    let (plain_pixels, _) = render(&mut device, &plain);
    let (doubled_pixels, _) = render(&mut device, &doubled);
    assert_eq!(
        plain_pixels, doubled_pixels,
        "a scale that cancels changed the ramp, so the command's transform reached the \
         paint"
    );
}

/// §8.7.4.1's "the geometry of the gradient fill is independent of that of the object
/// being painted", drawn as two *separate* marks: one shading, two shapes, and the sweep
/// continues across the gap rather than restarting in each.
#[test]
fn one_shading_sweeps_continuously_across_two_separate_marks() {
    let mut device = device();
    let ramp = grey_ramp(&mut device);
    let left = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(4.0, 24.0),
            Point::new(20.0, 40.0),
        )))
        .expect("upload");
    let right = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(44.0, 24.0),
            Point::new(60.0, 40.0),
        )))
        .expect("upload");
    let paint = page_wide_ramp(ramp);
    let mut builder = SceneBuilder::new();
    for outline in [left, right] {
        builder
            .fill(
                outline,
                Affine::IDENTITY,
                FillRule::NonZero,
                paint,
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .expect("a valid shaded fill");
    }
    let (pixels, _) = render(&mut device, &builder.finish());

    for x in [5_u32, 19, 45, 59] {
        let probe = pixel(&pixels, SIZE, x, 32);
        assert_eq!(probe[3], 255, "column {x} is inside one of the two marks");
        let expected = clause_grey_at(x);
        assert!(
            i32::from(probe[0]).abs_diff(i32::from(expected)) <= 1,
            "column {x}: {} where the page-anchored ramp gives {expected}; a paint that \
             restarted per shape would read near 0 at each mark's left edge",
            probe[0]
        );
    }
    assert_eq!(
        pixel(&pixels, SIZE, 32, 32)[3],
        0,
        "the gap between the two marks is unpainted: a shading paints where the mark is, \
         even though it is measured from the page"
    );
}

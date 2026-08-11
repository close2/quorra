//! What the glyph atlas will hold, and what it must never confuse with something else.
//!
//! ADR 0024. The cache used to admit a tile by a **dimension** — 128 pixels a side —
//! while the thing it protected was a **budget**: one zoomed-in letterform must not
//! evict a page's worth of text. Those are not the same rule, and the mismatch was a
//! cliff: past about 10× magnification every visible letterform left the atlas and was
//! rasterised again on every frame, so a frame drawing thirty glyphs cost ten times one
//! drawing 5 933.
//!
//! Replacing it uncovered a defect the old bound had been hiding, which is the second
//! subject here: **the key had no fill rule in it**. The same outline under non-zero and
//! under even-odd is two different pictures wherever a subpath nests, and the cache
//! handed the first to the second. Only shapes under 128 pixels could reach it, and the
//! test that would have caught it used a 140-pixel shape.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, Paint, Point, Rect, Scene, SceneBuilder, Segment,
};

const SIZE: u32 = 320;

fn device(atlas_budget: u64) -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        atlas_budget,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

fn render(device: &mut Device, scene: &Scene) -> (Vec<u8>, u32) {
    let frame = device
        .render(
            scene,
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders");
    let tiles = frame.counters().tiles;
    (frame.into_raster().unwrap().into_pixels(), tiles)
}

fn rect_ring(outer: f32, inner: f32) -> Vec<Segment> {
    let ring = |a: f32, b: f32| {
        vec![
            Segment::MoveTo(Point::new(a, a)),
            Segment::LineTo(Point::new(b, a)),
            Segment::LineTo(Point::new(b, b)),
            Segment::LineTo(Point::new(a, b)),
            Segment::Close,
        ]
    };
    // Both subpaths wound the same way: filled solid under non-zero, holed under
    // even-odd (§8.5.3.3).
    let mut path = ring(20.0, outer);
    path.extend(ring(inner, outer - (inner - 20.0)));
    path
}

fn fill_scene(outline: quorra_scene::OutlineId, rule: FillRule) -> Scene {
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            rule,
            Paint::Solid(Color::new(0.0, 0.6, 0.0, 1.0)),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("valid fill");
    builder.finish()
}

/// The rule is part of what a tile *is*, so it must be part of the key. Small enough
/// that both fills are cached, which is exactly the case the old dimension bound hid.
#[test]
fn the_fill_rule_is_part_of_the_key() {
    let mut device = device(quorra_gpu::DEFAULT_ATLAS_BUDGET);
    let outline = device.upload_outline(&rect_ring(90.0, 40.0)).unwrap();

    let (nonzero, cached) = render(&mut device, &fill_scene(outline, FillRule::NonZero));
    assert_eq!(
        cached, 0,
        "a 70-pixel tile belongs in the atlas, not the sheet"
    );
    let (evenodd, _) = render(&mut device, &fill_scene(outline, FillRule::EvenOdd));

    // The middle of the nested subpath: solid under non-zero, a hole under even-odd.
    let at = ((55 * SIZE + 55) * 4 + 1) as usize;
    assert_eq!(
        nonzero[at], 153,
        "non-zero fills the nested subpath (§8.5.3.3.2)"
    );
    assert_eq!(
        evenodd[at], 0,
        "even-odd holes it (§8.5.3.3.3) — a cache keyed without the rule returns the \
         non-zero tile here, which is what ADR 0024 found"
    );

    // And the other order, so the test cannot pass by the first render winning a race.
    let (evenodd_first, _) = render(&mut device, &fill_scene(outline, FillRule::EvenOdd));
    assert_eq!(evenodd_first[at], 0);
}

/// A tile is admitted by what it would take of *this* atlas, not by a constant: the
/// same shape is cached against a large budget and reaches the sheet against a small
/// one.
#[test]
fn admission_follows_the_budget_and_not_a_dimension() {
    let shape = rect_ring(200.0, 60.0); // ~180 px a side, past the old 128 bound
    let mut roomy = device(quorra_gpu::DEFAULT_ATLAS_BUDGET);
    let outline = roomy.upload_outline(&shape).unwrap();
    let (_, tiles) = render(&mut roomy, &fill_scene(outline, FillRule::NonZero));
    assert_eq!(
        tiles, 0,
        "180 pixels a side is 32 KB against an 8 MiB atlas: cached, where the old \
         128-pixel bound sent it to the sheet on every frame"
    );

    // An eighth of 64 KiB is 8 KB, which the same tile exceeds.
    let mut tight = device(64 * 1024);
    let outline = tight.upload_outline(&shape).unwrap();
    let (_, tiles) = render(&mut tight, &fill_scene(outline, FillRule::NonZero));
    assert_eq!(
        tiles, 1,
        "against a small atlas the same tile takes the sheet — and `Counters::tiles` \
         is what says so, which it did not until ADR 0024"
    );
}

/// Uncached and cached are the same picture: the rule decides where a tile lives, not
/// what it contains.
#[test]
fn the_two_paths_draw_the_same_pixels() {
    let shape = rect_ring(200.0, 60.0);
    let mut roomy = device(quorra_gpu::DEFAULT_ATLAS_BUDGET);
    let outline = roomy.upload_outline(&shape).unwrap();
    let (cached, _) = render(&mut roomy, &fill_scene(outline, FillRule::EvenOdd));

    let mut tight = device(64 * 1024);
    let outline = tight.upload_outline(&shape).unwrap();
    let (sheeted, _) = render(&mut tight, &fill_scene(outline, FillRule::EvenOdd));

    assert_eq!(
        cached, sheeted,
        "one rasteriser feeds both paths, so admission may not change a pixel"
    );
}

/// `Counters::tiles` counts what the sheet holds, for both lanes and every shape that
/// reaches it — the instrument that read zero for three milestones.
#[test]
fn the_tile_counter_counts() {
    let mut device = device(64 * 1024);
    let mut builder = SceneBuilder::new();
    for i in 0..3_u32 {
        let offset = i as f32 * 4.0;
        let outline = device
            .upload_outline(&rect_ring(200.0 - offset, 60.0))
            .unwrap();
        builder
            .fill(
                outline,
                Affine::IDENTITY,
                FillRule::NonZero,
                Paint::Solid(Color::new(0.0, 0.6, 0.0, 1.0)),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
    }
    let (_, tiles) = render(&mut device, &builder.finish());
    assert_eq!(tiles, 3, "three shapes past the atlas are three tiles");

    // A page of rectangles packs nothing: the analytic lane never touches the sheet.
    let mut builder = SceneBuilder::new();
    builder
        .rect(
            Rect::new(Point::new(4.0, 4.0), Point::new(60.0, 60.0)),
            Affine::IDENTITY,
            Color::new(0.2, 0.2, 0.2, 1.0),
            None,
            None,
        )
        .unwrap();
    let (_, tiles) = render(&mut device, &builder.finish());
    assert_eq!(tiles, 0, "and a rectangle is not a tile");
}

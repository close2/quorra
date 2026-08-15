//! A clip chain's residue is one region, and which mark is asking must not change it
//! (ADR 0049).
//!
//! ISO 32000-2 §8.5.4 makes a chain one region arrived at by intersecting paths, and
//! ADR 0030 already took that as far as *how the links combine*. What is checked here is
//! the other half: the region does not depend on the tile a command cuts out of it, so
//! rasterising it once and cutting every tile from it must draw the page the per-tile
//! path draws.
//!
//! **Both paths are exercised in one binary and on one device**, by moving
//! `Options::max_frame_bytes` — which is what the regions' budget is a quarter of — so
//! that the same scene is drawn once with the region kept and once with every command
//! paying for its own tile. Two builds compared afterwards would be comparing two
//! compilations; this compares two calls.

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

use quorra_gpu::{Counters, Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, OutlineId, Paint, Point, Scene, SceneBuilder,
    Segment,
};

/// 64 pixels wide: 64 × 4 bytes = 256, the buffer-copy row alignment.
const SIZE: u32 = 64;

fn device_with(max_frame_bytes: u64) -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        max_frame_bytes,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

fn render(device: &mut Device, scene: &Scene) -> (Vec<u8>, Counters) {
    let frame = device
        .render(
            scene,
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the frame draws");
    let counters = frame.counters();
    (frame.into_raster().unwrap().into_pixels(), counters)
}

/// A closed curve — not a rectangle under any transform, so it stays a residue link.
fn blob(device: &mut Device, radius: f32) -> OutlineId {
    let mut path = vec![Segment::MoveTo(Point::new(-radius, 0.0))];
    for step in 0..8_u8 {
        let from = f32::from(step) / 8.0 * std::f32::consts::TAU;
        let to = f32::from(step + 1) / 8.0 * std::f32::consts::TAU;
        let at = |angle: f32| Point::new(radius * angle.cos(), radius * angle.sin() * 1.2);
        let (a, b) = (at(from), at(to));
        path.push(Segment::CubicTo {
            c1: Point::new(a.x + (b.x - a.x) * 0.35, a.y + (b.y - a.y) * 0.1),
            c2: Point::new(a.x + (b.x - a.x) * 0.65, a.y + (b.y - a.y) * 0.9),
            to: b,
        });
    }
    path.push(Segment::Close);
    device.upload_outline(&path).expect("an outline")
}

/// Four marks under one curved clip: the shape the region exists for, small enough that
/// a whole page of it is a readback a test can afford.
fn four_marks_under_one_clip(device: &mut Device) -> Scene {
    let clip_outline = blob(device, 26.0);
    let mark = blob(device, 14.0);
    let mut builder = SceneBuilder::new();
    let clip = builder
        .clip(
            clip_outline,
            Affine::translate(32.0, 32.0),
            FillRule::NonZero,
            None,
        )
        .expect("a curved clip");
    for (x, y) in [(20.0, 20.0), (44.0, 20.0), (20.0, 44.0), (44.0, 44.0)] {
        builder
            .fill(
                mark,
                Affine::translate(x, y),
                FillRule::NonZero,
                Paint::Solid(Color::new(0.1, 0.2, 0.9, 1.0)),
                Some(clip),
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .expect("a clipped fill");
    }
    builder.finish()
}

/// **The picture is the same whether the region was kept or every tile paid for itself.**
///
/// The bound is not zero and the reason is stated in ADR 0049: the region's prefix sum
/// crosses the columns left of a tile one at a time where the tile takes them as one
/// deposit at its border, and `f32` addition is not associative. `raster.rs`'s
/// `a_tile_is_the_crop_of_the_region_that_contains_it` measures that at 1 of 255 on 31
/// pixels in 2.9 million; this is the same statement after the coverage has been through
/// the compositor, where a 1-of-255 coverage difference can move a channel by 1.
#[test]
fn keeping_the_region_does_not_change_the_page() {
    let mut kept = device_with(Options::default().max_frame_bytes);
    let scene = four_marks_under_one_clip(&mut kept);
    let (with_region, region_counters) = render(&mut kept, &scene);

    // A budget whose quarter is smaller than the clip's region (about 52 × 63 pixels),
    // and far larger than everything else this frame allocates.
    let mut per_tile = device_with(8_192);
    let scene = four_marks_under_one_clip(&mut per_tile);
    let (with_tiles, tile_counters) = render(&mut per_tile, &scene);

    assert_eq!(
        (
            region_counters.clip_residue_regions,
            region_counters.clip_residue_tiles
        ),
        (1, 0),
        "one chain, one region, and no command paying for its own"
    );
    assert_eq!(
        (
            tile_counters.clip_residue_regions,
            tile_counters.clip_residue_tiles
        ),
        (0, 4),
        "the budget declined the region, so all four commands pay per tile"
    );

    let worst = with_region
        .iter()
        .zip(&with_tiles)
        .map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs())
        .max()
        .unwrap_or(0);
    let differing = with_region
        .iter()
        .zip(&with_tiles)
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        worst <= 1,
        "the two paths differ by {worst} of 255 on {differing} of {} bytes, which is not \
         the accumulator's rounding",
        with_region.len()
    );
}

/// **A frame whose regions do not fit their budget is drawn, not refused** — the cache
/// declines, and only a *frame* refuses (ADR 0049, ADR 0029's precedent).
///
/// The same scene at a budget whose quarter cannot hold one region: every command
/// rasterises the chain over its own tile, which is what every frame did before the
/// regions existed, and the counters say so rather than leaving it to a clock.
#[test]
fn a_budget_too_small_for_a_region_still_draws_the_frame() {
    let mut device = device_with(8_192);
    let scene = four_marks_under_one_clip(&mut device);
    let (pixels, counters) = render(&mut device, &scene);
    assert_eq!(counters.clip_residue_regions, 0, "no region was allocated");
    assert_eq!(counters.clip_residue_tiles, 4, "four tiles paid instead");
    assert!(
        pixels.iter().any(|byte| *byte != 0),
        "a declined region must not become a blank page"
    );
}

/// **The counter counts distinct regions, not lookups.** Forty marks under one chain are
/// one region; forty marks under forty chains are forty.
///
/// The instrument this project insists on: a hit rate would read 100 % for both of these,
/// and the second is the page that pays forty times.
#[test]
fn the_counter_is_of_regions_and_not_of_the_commands_that_ask() {
    let mut device = device_with(Options::default().max_frame_bytes);
    let clip_outline = blob(&mut device, 26.0);
    let mark = blob(&mut device, 5.0);

    let mut shared = SceneBuilder::new();
    let one = shared
        .clip(
            clip_outline,
            Affine::translate(32.0, 32.0),
            FillRule::NonZero,
            None,
        )
        .expect("a curved clip");
    let mut many = SceneBuilder::new();
    let mut separate = Vec::new();
    for i in 0..40_u8 {
        separate.push(
            many.clip(
                clip_outline,
                // Each clip a pixel along, so no two chains are the same region.
                Affine::translate(32.0 + f32::from(i) * 0.0, 32.0 + f32::from(i) * 0.25),
                FillRule::NonZero,
                None,
            )
            .expect("a curved clip"),
        );
    }
    for i in 0..40_u8 {
        let at = Affine::translate(20.0 + f32::from(i % 8) * 3.0, 20.0 + f32::from(i / 8) * 3.0);
        let paint = Paint::Solid(Color::new(0.1, 0.2, 0.9, 1.0));
        shared
            .fill(
                mark,
                at,
                FillRule::NonZero,
                paint,
                Some(one),
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .expect("a clipped fill");
        many.fill(
            mark,
            at,
            FillRule::NonZero,
            paint,
            Some(separate[i as usize]),
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("a clipped fill");
    }

    let shared = shared.finish();
    let many = many.finish();
    let (_, one_chain) = render(&mut device, &shared);
    let (_, forty_chains) = render(&mut device, &many);
    assert_eq!(one_chain.clip_residue_regions, 1, "40 commands, one region");
    assert_eq!(one_chain.clip_residue_tiles, 0);
    assert_eq!(
        (
            forty_chains.clip_residue_regions,
            forty_chains.clip_residue_tiles
        ),
        (0, 40),
        "forty chains each used once, and a region that costs more than the tile it \
         would replace is not kept — the rule, on the page it exists for"
    );
}

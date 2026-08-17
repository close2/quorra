//! What the atlas budget buys, and what a frame pays when the atlas is full.
//!
//! ADR 0063, and the two states it exists to tell apart. From outside a `Frame`, "this
//! page is larger than the atlas" and "the atlas is full of the page before it" looked
//! identical: `atlas_working_set_bytes` said how large the page was, `tiles` mixed every
//! lane's output together, and nothing said how much of *this* frame the cache declined.
//! Over the caller's corpus at 4× the second state accounts for 74 820 marks on 19 of 948
//! pages and the first for none at all — the largest single page asks for 4.10 MiB of an
//! 8 MiB atlas (`doc/notes-atlas-budget.md`).
//!
//! The other subject is one word in `Options`: `atlas_budget` is a **request**. The
//! texture is near-square with its width capped at 2048 and its sides clamped to the
//! adapter's limit, so a large enough budget is granted in part, silently and correctly —
//! [`Limits::atlas_bytes`] is what says so.
//!
//! [`Limits::atlas_bytes`]: quorra_gpu::device::Limits::atlas_bytes

// Test-file lint policy as in m1.rs, plus the grid arithmetic below: `i` is bounded by a
// literal loop count of at most 60 and the factors are literals, so nothing here can wrap.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects
)]

use quorra_gpu::frame::Counters;
use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, OutlineId, Paint, Point, Scene, SceneBuilder,
    Segment,
};

const SIZE: u32 = 320;

/// The atlas's width can never exceed this, whatever the budget says (`AtlasStore::new`).
const ATLAS_WIDTH_CAP: u64 = 2048;

fn device(atlas_budget: u64) -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        atlas_budget,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

/// A letterform-sized outline of `side` device pixels — small enough that admission never
/// refuses it, so the only thing that can keep it out of the atlas is room.
///
/// **A triangle and not a square**, which is not decoration: four axis-aligned corners are
/// `rect_hint`'s shape and take the analytic rectangle lane (ADR 0047), which never asks
/// the atlas anything. The first version of this file used squares and every assertion
/// below read zero — `HANDOVER.md`'s "a fixture that names a lane should say which lane it
/// means", met head on.
fn pip(side: f32) -> Vec<Segment> {
    vec![
        Segment::MoveTo(Point::new(0.0, 0.0)),
        Segment::LineTo(Point::new(side, 0.0)),
        Segment::LineTo(Point::new(side * 0.5, side)),
        Segment::Close,
    ]
}

/// `count` distinct outlines, each a slightly different square, so every placement is its
/// own atlas key and the page's working set grows with `count`.
fn distinct_keys(device: &mut Device, count: u32) -> Scene {
    let mut builder = SceneBuilder::new();
    for i in 0..count {
        let outline: OutlineId = device.upload_outline(&pip(12.0 + (i % 5) as f32)).unwrap();
        builder
            .fill(
                outline,
                Affine::translate(((i % 16) * 18) as f32, ((i / 16) * 18) as f32),
                FillRule::NonZero,
                Paint::Solid(Color::new(0.0, 0.6, 0.0, 1.0)),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
    }
    builder.finish()
}

fn render(device: &mut Device, scene: &Scene) -> (Vec<u8>, Counters) {
    let frame = device
        .render(
            scene,
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("a page the atlas cannot hold still draws");
    let counters = frame.counters();
    (frame.into_raster().unwrap().into_pixels(), counters)
}

/// **`Options::atlas_budget` is a request and `Limits::atlas_bytes` is what it bought.**
///
/// Two directions, because a field that merely echoed the request would pass the first
/// alone: an ordinary budget is granted whole, and a budget past `2048 × max_target_size`
/// is granted in part with nothing failing.
#[test]
fn the_budget_is_a_request_and_the_limit_is_what_it_bought() {
    let ordinary = device(quorra_gpu::DEFAULT_ATLAS_BUDGET);
    let ceiling = ATLAS_WIDTH_CAP.saturating_mul(u64::from(ordinary.limits().max_target_size));
    assert!(
        quorra_gpu::DEFAULT_ATLAS_BUDGET <= ceiling,
        "this assertion's premise: the default is inside the cap on this adapter, so the \
         next one is about the sizing and not about the clamp"
    );
    assert_eq!(
        ordinary.limits().atlas_bytes,
        quorra_gpu::DEFAULT_ATLAS_BUDGET,
        "a budget inside the cap is granted whole"
    );

    // Four times the largest atlas any adapter could give us.
    let greedy = device(ceiling.saturating_mul(4));
    assert_eq!(
        greedy.limits().atlas_bytes,
        ceiling,
        "the width cap and the adapter's texture limit bound the atlas together, so a \
         budget past their product buys nothing more — the silence ADR 0063 found"
    );
    assert!(
        greedy.limits().atlas_bytes < ceiling.saturating_mul(4),
        "and the caller asked for more than it got, which is the whole reason this field \
         exists: `atlas_working_set_bytes` compared against the request would be compared \
         against a number no texture has"
    );
}

/// **A frame says how many marks the full atlas cost it**, and says nothing when it cost
/// nothing.
///
/// The same page against two atlases. The pixels are equal either way — one rasteriser
/// feeds both paths — so `atlas_overflow_tiles` is the only observable that moves, which
/// is exactly why it had to exist.
#[test]
fn a_frame_says_how_many_marks_the_full_atlas_cost_it() {
    let mut roomy = device(quorra_gpu::DEFAULT_ATLAS_BUDGET);
    let scene = distinct_keys(&mut roomy, 60);
    let (cached, counters) = render(&mut roomy, &scene);
    assert!(
        counters.atlas_distinct_keys >= 60,
        "the fixture's premise: 60 placements are 60 keys, so the atlas is being asked \
         for something ({counters:?})"
    );
    assert_eq!(
        counters.atlas_overflow_tiles, 0,
        "an atlas with room declines nothing ({counters:?})"
    );

    let mut tight = device(4 * 1024);
    let scene = distinct_keys(&mut tight, 60);
    let (sheeted, counters) = render(&mut tight, &scene);
    assert!(
        counters.atlas_overflow_tiles > 0,
        "against a 4 KiB atlas most of these tiles have nowhere to go, and the counter is \
         what says how many ({counters:?})"
    );
    assert_eq!(
        counters.atlas_overflow_tiles + counters.atlas_entries,
        60,
        "every key either found an entry or was declined, and there is no third state \
         ({counters:?})"
    );
    assert_eq!(
        cached, sheeted,
        "and admission may not change a pixel (ADR 0024's standing assertion)"
    );
}

/// **The two reasons a mark goes uncached, told apart by the pair of counters.**
///
/// This is what ADR 0063 is for. `atlas_overflow_tiles` alone cannot distinguish them and
/// neither can `atlas_working_set_bytes` alone; together they are exact.
#[test]
fn a_page_too_large_and_an_atlas_holding_another_page_are_told_apart() {
    let atlas = 4 * 1024;

    // (a) The page is larger than the atlas. Working set **over** the limit.
    let mut alone = device(atlas);
    let scene = distinct_keys(&mut alone, 60);
    let (_, counters) = render(&mut alone, &scene);
    assert!(
        counters.atlas_working_set_bytes > alone.limits().atlas_bytes
            && counters.atlas_overflow_tiles > 0,
        "a page whose own distinct keys outgrow the atlas: raising the budget is the \
         answer, and this is the pair that says so ({counters:?}, atlas {})",
        alone.limits().atlas_bytes
    );

    // (b) The atlas is full of an earlier page. Working set **under** the limit, and the
    // marks are declined anyway — the state that is 74 820 marks of the caller's corpus
    // and that no counter could name before this ADR.
    let mut shared = device(atlas);
    let earlier = distinct_keys(&mut shared, 60);
    render(&mut shared, &earlier);
    let later = distinct_keys(&mut shared, 4);
    let (_, counters) = render(&mut shared, &later);
    assert!(
        counters.atlas_working_set_bytes <= shared.limits().atlas_bytes,
        "the fixture's premise: this page fits the atlas by bytes ({counters:?}, atlas {})",
        shared.limits().atlas_bytes
    );
    assert!(
        counters.atlas_overflow_tiles > 0,
        "and is declined anyway, because the sheet is full of the page before it — no \
         budget reaches this, and the repack that follows the frame is what clears it \
         ({counters:?})"
    );
}

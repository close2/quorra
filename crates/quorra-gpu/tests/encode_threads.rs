//! A frame drawn on many threads is the frame drawn on one — the same bytes, the same
//! counters, and the same refusal.
//!
//! `Options::encode_threads` divides the one part of `encode` that is a pure function of
//! a mark's own geometry (`encode/parallel.rs`). Everything the frame's order depends on
//! stays on the calling thread, so byte equality is a property the design **has** rather
//! than a tolerance it approximates — and the caller's `doc/QUORRA_ENCODE_THREADS.md` §5
//! asks for exactly that, in a tree whose own corpus gate holds four lanes to equality
//! and would find a violation after we shipped it.
//!
//! Every test here therefore states its claim as equality against the one-threaded
//! frame, at several thread counts including one far above what the divided work needs.
//! The fixtures are chosen so that a *missing* one of the encoder's drain points would
//! move a pixel: marks overlap, so draw order is visible; a rectangle, a stroke, a
//! blended fill, a curve-clipped fill and a group sit between runs of fills, so a queue
//! that survived one of them would reorder the page.
//!
//! **One drain point is not among them, and this file cannot reach it.** Every op
//! `busy_page` pushes follows a `plan_child` that has drained already, so
//! `Encoder::push_op`'s own drain can be deleted and all four tests below go on passing —
//! measured by forcing exactly that, 2026-08-17. Reaching it needs a **rare-lane** command
//! (an image, a shading, a function paint) in the middle of a run of queued fills, which
//! is what `tests/encode_threads_nested.rs` puts there. An earlier version of this
//! paragraph claimed "an image-free rare paint" was already among the fixtures; there is
//! no rare paint in this file at all.
//!
//! `llvmpipe` by name, as most of this suite does, so CI on a software rasteriser reads
//! the same numbers.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Counters, Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, GroupSpec, LineCap, LineJoin, OutlineId, Paint,
    Point, Rect, Scene, SceneBuilder, Segment, Stroke,
};

const SIDE: u32 = 220;

/// The counts every equality below is asserted at: one, two that must split the work,
/// an odd count that cannot divide any of these fixtures evenly, and far more threads
/// than there is work for.
const COUNTS: [usize; 5] = [1, 2, 3, 7, 64];

fn device(threads: usize) -> Device {
    let device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        encode_threads: threads,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs");
    device.wait_until_warm();
    device
}

fn ink(shade: f32) -> Paint {
    Paint::Solid(Color::new(shade, 0.2, 1.0 - shade, 0.85))
}

/// A closed curve of `lobes` cubics, `radius` across — a letterform for costing
/// purposes, and small enough at `radius = 4` that the atlas admits it.
fn blob(lobes: u32, radius: f32) -> Vec<Segment> {
    let point = |angle: f32, r: f32| Point::new(r * angle.cos(), r * angle.sin());
    let mut path = vec![Segment::MoveTo(point(0.0, radius))];
    for step in 0..lobes {
        let from = f32::from(step as u16) / lobes as f32 * std::f32::consts::TAU;
        let to = f32::from(step as u16 + 1) / lobes as f32 * std::f32::consts::TAU;
        let (a, b) = (point(from, radius), point(to, radius * 0.8));
        path.push(Segment::CubicTo {
            c1: Point::new(a.x + (b.x - a.x) * 0.3, a.y + (b.y - a.y) * 0.1),
            c2: Point::new(a.x + (b.x - a.x) * 0.7, a.y + (b.y - a.y) * 0.9),
            to: b,
        });
    }
    path.push(Segment::Close);
    path
}

fn square(half: f32) -> Vec<Segment> {
    vec![
        Segment::MoveTo(Point::new(-half, -half)),
        Segment::LineTo(Point::new(half, -half)),
        Segment::LineTo(Point::new(half, half)),
        Segment::LineTo(Point::new(-half, half)),
        Segment::Close,
    ]
}

/// Where a mark goes: a tight lattice, on purpose.
///
/// The step is a quarter of the largest mark, so every mark overlaps several of its
/// neighbours and every mark is drawn at less than full alpha. That is what makes draw
/// **order** visible in the pixels — a page whose marks do not touch is a page that
/// compares equal however it was reordered, and a gate that cannot fail is not a gate
/// (`doc/HANDOVER.md`).
fn at(index: u32) -> Affine {
    let columns = 6;
    let x = 30.0 + f32::from((index % columns) as u16) * 6.0;
    let y = 30.0 + f32::from((index / columns) as u16) * 6.0;
    Affine::translate(x, y)
}

/// A page whose marks **overlap** and whose runs of fills are interrupted by every other
/// kind of command the walk has: a rectangle instance, a stroke, a blended fill (which
/// becomes an implicit one-element group), a real group, and a curve-clipped fill (which
/// takes the residue path the fan-out declines).
///
/// The overlap is what makes draw order visible in the pixels: every mark is drawn at
/// 0.85 alpha, so a pair composited in the wrong sequence is a different colour, not the
/// same one.
/// The outlines `busy_page` draws with: one the atlas admits, one it will not, and one
/// the rectangle lane recognises.
struct Shapes {
    small: OutlineId,
    large: OutlineId,
    boxed: OutlineId,
}

/// One ordinary fill — the case the fan-out exists for, and the run every other arm
/// interrupts.
fn plain_fill(builder: &mut SceneBuilder, outline: OutlineId, index: u32, shade: f32) {
    builder
        .fill(
            outline,
            at(index),
            FillRule::NonZero,
            ink(shade),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
}

/// A stroke, which the fan-out also takes: expansion and a fill, both pure functions of
/// this command's own outline.
fn stroked(builder: &mut SceneBuilder, outline: OutlineId, index: u32, shade: f32) {
    builder
        .stroke(
            outline,
            at(index),
            Stroke {
                width: 2.0,
                adjust: false,
                cap: LineCap::Round,
                join: LineJoin::Miter,
                miter_limit: 4.0,
            },
            ink(shade),
            None,
            BlendMode::Normal,
            None,
        )
        .unwrap();
}

/// A group, which switches the plan under the walk — the boundary `plan_child` drains at,
/// twice.
fn grouped(builder: &mut SceneBuilder, small: OutlineId, index: u32, shade: f32) {
    builder
        .group(
            GroupSpec {
                alpha: 0.7,
                blend: BlendMode::Normal,
                clip: None,
                knockout: false,
                mask: None,
                isolated: true,
                compose: Compose::SrcOver,
            },
            |body| {
                for inner in 0..4 {
                    body.fill(
                        small,
                        at(index + inner),
                        FillRule::NonZero,
                        ink(shade),
                        None,
                        BlendMode::Normal,
                        Compose::SrcOver,
                        None,
                    )?;
                }
                Ok(())
            },
        )
        .unwrap();
}

/// One command of the busy page, chosen so that every lane the walk has appears between
/// runs of ordinary fills.
fn busy_command(
    builder: &mut SceneBuilder,
    shapes: &Shapes,
    clip: quorra_scene::ClipId,
    index: u32,
) {
    let shade = f32::from((index % 7) as u16) / 7.0;
    match index % 15 {
        // A rectangle instance: a lane that emits while a run of fills may be queued.
        3 => builder
            .rect(
                Rect::new(Point::new(10.0, 10.0), Point::new(30.0, 20.0)),
                at(index),
                Color::new(shade, 0.6, 0.1, 0.5),
                None,
                None,
            )
            .unwrap(),
        5 => stroked(builder, shapes.large, index, shade),
        // A blended fill: §11.3.5's implicit one-element group, so `plan_child` runs in
        // the middle of a run.
        7 => builder
            .fill(
                shapes.large,
                at(index),
                FillRule::NonZero,
                ink(shade),
                None,
                BlendMode::Multiply,
                Compose::SrcOver,
                None,
            )
            .unwrap(),
        // A clipped fill: the residue path, which stays on the walk's thread.
        9 => builder
            .fill(
                shapes.large,
                at(index),
                FillRule::NonZero,
                ink(shade),
                Some(clip),
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap(),
        11 => grouped(builder, shapes.small, index, shade),
        // A rect-hinted fill, which takes ADR 0047's analytic lane.
        13 => plain_fill(builder, shapes.boxed, index, shade),
        // Everything else is an ordinary fill: the atlas takes the small one and the
        // sheet takes the large one, so both deferred lanes are populated.
        other if other % 2 == 0 => plain_fill(builder, shapes.small, index, shade),
        _ => plain_fill(builder, shapes.large, index, shade),
    }
}

fn busy_page(device: &mut Device) -> Scene {
    let shapes = Shapes {
        small: device.upload_outline(&blob(6, 4.0)).unwrap(),
        large: device.upload_outline(&blob(24, 22.0)).unwrap(),
        boxed: device.upload_outline(&square(9.0)).unwrap(),
    };
    let mut builder = SceneBuilder::new();
    let clip = builder
        .clip(
            shapes.large,
            Affine::translate(60.0, 60.0),
            FillRule::NonZero,
            None,
        )
        .unwrap();
    for index in 0..90_u32 {
        busy_command(&mut builder, &shapes, clip, index);
    }
    builder.finish()
}

/// A page of one outline placed at one transform, over and over: every placement shares
/// an atlas key, so this is the guard in `encode/parallel.rs` — two queued jobs may not
/// rasterise one key — asked for directly.
fn repeated_key(device: &mut Device) -> Scene {
    let small = device.upload_outline(&blob(6, 4.0)).unwrap();
    let mut builder = SceneBuilder::new();
    for index in 0..200_u32 {
        builder
            .fill(
                small,
                // Whole pixels and one transform: the key is identical every time.
                Affine::translate(20.0, 20.0),
                FillRule::NonZero,
                ink(f32::from((index % 5) as u16) / 5.0),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
    }
    builder.finish()
}

fn draw(threads: usize, scene: impl Fn(&mut Device) -> Scene) -> (Vec<u8>, Counters) {
    let mut device = device(threads);
    let scene = scene(&mut device);
    let frame = device
        .render(
            &scene,
            &Viewport::full(SIDE, SIDE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the fixture is inside every budget");
    let counters = frame.counters();
    (frame.into_raster().unwrap().into_pixels(), counters)
}

/// §4.6, and the caller's §5: *a frame drawn on 24 threads must be the same bytes as the
/// same frame drawn on one.*
#[test]
fn a_busy_page_is_the_same_bytes_at_every_thread_count() {
    let (alone, counters) = draw(1, busy_page);
    assert!(
        alone.iter().skip(3).step_by(4).any(|&a| a > 0),
        "the fixture draws something, or every equality below is between two blank pages"
    );
    assert!(
        counters.tiles > 0 && counters.atlas_distinct_keys > 0,
        "the fixture populates both deferred lanes: {counters:?}"
    );
    for threads in COUNTS.into_iter().skip(1) {
        let (divided, also) = draw(threads, busy_page);
        assert_eq!(
            counters, also,
            "the counters moved between 1 thread and {threads}"
        );
        assert!(
            divided == alone,
            "the page drawn on {threads} threads is not the page drawn on one"
        );
    }
}

/// The atlas guard: a run of placements that all share one key rasterises it once,
/// whatever the thread count, and draws the same page.
#[test]
fn a_repeated_atlas_key_is_rasterised_once_at_every_thread_count() {
    let (alone, counters) = draw(1, repeated_key);
    assert_eq!(
        counters.atlas_distinct_keys, 1,
        "two hundred placements of one outline at one transform are one key"
    );
    for threads in COUNTS.into_iter().skip(1) {
        let (divided, also) = draw(threads, repeated_key);
        assert_eq!(counters, also, "the counters moved at {threads} threads");
        assert!(divided == alone, "the page moved at {threads} threads");
    }
}

/// **The refusals must not move** (the caller's §5): a frame that exceeds its budget
/// refuses with the same variant and the same two numbers on any thread count, because
/// the commit charges in encounter order whoever rasterised.
#[test]
fn a_budget_refusal_names_the_same_numbers_at_every_thread_count() {
    let refusal = |threads: usize| {
        let mut device = Device::headless(&Options {
            adapter: Some("llvmpipe".into()),
            encode_threads: threads,
            // Room for the instance streams the walk charges up front, and not for many
            // of the coverage tiles that follow — so the refusal happens *inside* the run
            // of fills rather than before it.
            max_frame_bytes: 40_000,
            ..Options::default()
        })
        .expect("llvmpipe is present wherever this suite runs");
        device.wait_until_warm();
        let scene = busy_page(&mut device);
        let error = device
            .render(
                &scene,
                &Viewport::full(SIDE, SIDE, Affine::IDENTITY),
                Target::Readback,
            )
            .expect_err("this budget cannot hold this page");
        format!("{error:?}")
    };
    let alone = refusal(1);
    assert!(
        alone.contains("FrameBudgetExceeded"),
        "the fixture must refuse on the budget, not on something else: {alone}"
    );
    for threads in COUNTS.into_iter().skip(1) {
        assert_eq!(
            alone,
            refusal(threads),
            "the refusal moved at {threads} threads"
        );
    }
}

/// A blank scene is a legitimate scene (principle 6), and a thread count does not make
/// one into an error.
#[test]
fn a_blank_scene_draws_the_same_nothing_at_every_thread_count() {
    let blank = |_: &mut Device| SceneBuilder::new().finish();
    let (alone, counters) = draw(1, blank);
    for threads in COUNTS.into_iter().skip(1) {
        let (divided, also) = draw(threads, blank);
        assert_eq!(counters, also);
        assert!(divided == alone, "a blank page moved at {threads} threads");
    }
}

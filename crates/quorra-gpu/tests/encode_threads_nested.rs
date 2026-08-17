//! The thread count reaches a **nested** context, and the frame it draws there is the
//! frame one thread draws.
//!
//! # Where the question comes from
//!
//! hayro #1316, by way of the caller's `doc/HAYRO_ISSUES_FOR_QUORRA.md` §7. Their summary
//! of it names the nesting explicitly:
//!
//! > `hayro::render` hardcodes `num_threads: 0` and re-forces it for nested contexts, so
//! > an embedder cannot opt into multithreaded rendering at all.
//!
//! `Options::encode_threads` exists here (ADR 0054) and `tests/encode_threads.rs` holds a
//! flat page to byte equality across thread counts. What that file cannot see is the half
//! this one is about: a page whose marks are inside groups inside groups, where the walk
//! crosses a plan boundary — and a drain — between every run. Two claims, and they are
//! different in kind:
//!
//! 1. **Nothing re-forces the count.** `Options::encode_threads` is clamped once, at
//!    device construction, and read from there; no code path lowers it for a child plan.
//!    That is a claim about the source and is gated as one, for the reason under
//!    "What cannot be observed" below.
//! 2. **A nested page draws the same bytes at every count.** That is a claim about the
//!    pixels and is gated as one, at five counts, against a fixture that is shown to be
//!    order-sensitive rather than assumed to be.
//!
//! # The fixture, and why it is built the way it is
//!
//! `doc/HANDOVER.md`: *a determinism fixture that does not overlap is not a determinism
//! fixture.* ADR 0054's first thread-count gate used a lattice where no two marks touched
//! and it passed with an ordering drain removed. So this file does not argue that its
//! marks overlap — [`the_nested_fixture_is_order_sensitive`] draws the same marks in the
//! opposite order and requires a different page, which is a property no fixture of
//! disjoint marks can have.
//!
//! The deepest plan carries [`HEAVY_MARKS`] × 402 segments = 4 824, above the
//! 4 096-segment floor `encode/parallel.rs` puts under the fan-out — so the run *inside*
//! the nesting is one the fan-out actually takes, rather than one it declines for being
//! small. Each of the twelve is at its own scale, because twelve placements of one outline
//! at one scale are one atlas key and eleven residents, and a resident job weighs nothing.
//!
//! One thing here is not about nesting and is worth naming, because it was found by
//! forcing a defect rather than by reading: the translucent image over the deepest level's
//! queued marks is the only fixture in the tree that gates **`push_op`'s** drain.
//! `tests/encode_threads.rs` passes with that drain removed — every op it pushes follows a
//! `plan_child` that drained already — so the rare lane's route to an order-dependent
//! effect had no gate at all.
//!
//! # What cannot be observed, and why that is deliberate
//!
//! There is no public counter for how a frame's geometry was divided, and there must not
//! be one: `tests/encode_threads.rs` asserts `Counters` equality across thread counts, so
//! a counter that recorded the division would break the assertion that is the point of the
//! whole design. The division is unobservable through the API **by construction**. That is
//! why claim 1 above is a source gate; the forced defect that verifies claim 2 is what
//! shows the queue was live inside the nesting, since with one thread there is no queue and
//! removing a drain changes nothing.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects
)]

use std::path::{Path, PathBuf};

use quorra_gpu::{Counters, Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, ClipId, Color, Compose, FillRule, GroupSpec, ImageFilter, ImageId,
    ImageSpec, OutlineId, Paint, Point, Scene, SceneBuilder, SceneError, Segment,
};

const SIDE: u32 = 200;

/// The counts every equality is asserted at, as in `tests/encode_threads.rs`: one, two
/// that must split the work, an odd count that divides nothing evenly, and far more
/// threads than there is work for. `Device` clamps the last to the machine's own
/// parallelism, which is the point of the clamp and not a weakness of the test.
const COUNTS: [usize; 5] = [1, 2, 3, 7, 64];

/// How deep the groups nest. Four is well inside `MAX_GROUP_DEPTH` and is three more plan
/// boundaries than a flat page has.
const DEPTH: u32 = 4;

/// Marks in the deepest plan. Twelve at 402 segments each is 4 824, above the fan-out's
/// 4 096-segment floor, so the run inside the nesting is divided rather than declined.
const HEAVY_MARKS: u32 = 12;

/// A closed curve of `lobes` cubics, `radius` across.
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

struct Shapes {
    /// 402 segments and forty-four pixels across, at a scale that differs by placement so
    /// that each of the twelve is its own atlas key and its own rasterisation — the
    /// fan-out therefore has twelve jobs of real weight rather than one and eleven
    /// residents.
    heavy: OutlineId,
    /// Small enough for the atlas, so the glyph lane is populated too.
    light: OutlineId,
    /// A curve clip, applied to one mark per level. A residue clip is the one thing the
    /// fan-out declines (`encode/parallel/commit.rs`'s `deferrable_bounds`), so it puts
    /// work on the walk's own thread *between* the runs the fan-out takes — and it is
    /// what packs the scratch sheet, whose shelf cursors are the order-dependent state
    /// ADR 0034 made load-bearing.
    clip: ClipId,
    /// A translucent image, drawn over the queued light marks of the deepest level.
    ///
    /// The rare lane reaches `Encoder::push_op` with a queue that no plan boundary has
    /// drained, and **that drain site is otherwise ungated**: `tests/encode_threads.rs`
    /// goes on passing with `push_op`'s drain removed, because every op it pushes follows
    /// a `plan_child` that drained already. Verified by forcing exactly that.
    image: ImageId,
}

/// Ink at less than full opacity, so that which mark went down first is visible.
fn ink(shade: f32) -> Color {
    Color::new(shade, 0.25, 1.0 - shade, 0.75)
}

/// Where the heavy marks go, and at what size.
///
/// An eight-pixel step for a forty-four-pixel mark, so every one covers most of its
/// neighbours; a one-per-cent scale step, so no two share an atlas key and all twelve are
/// rasterised rather than eleven of them being resident copies of the first.
fn heavy_at(index: u32) -> Affine {
    let scale = 1.0 + f32::from(index as u16) * 0.01;
    Affine::scale(scale, scale).then(Affine::translate(
        70.0 + f32::from((index % 4) as u16) * 8.0,
        70.0 + f32::from((index / 4) as u16) * 8.0,
    ))
}

fn fill(
    builder: &mut SceneBuilder,
    outline: OutlineId,
    at: Affine,
    shade: f32,
    clip: Option<ClipId>,
) -> Result<(), SceneError> {
    builder.fill(
        outline,
        at,
        FillRule::NonZero,
        Paint::Solid(ink(shade)),
        clip,
        BlendMode::Normal,
        Compose::SrcOver,
        None,
    )
}

/// The run the fan-out divides, in encounter order or reversed.
///
/// Each mark keeps its own position and shade under reversal — only the *order* changes —
/// which is what makes the reversed page a statement about compositing order rather than
/// about where the marks are.
fn heavy_run(
    builder: &mut SceneBuilder,
    shapes: &Shapes,
    reversed: bool,
) -> Result<(), SceneError> {
    // Before the run and never inside the reversal, so that the reversed page differs
    // from the forward one because of the *marks* and not because of where this landed.
    // The light marks of this level are already queued when it is reached, which is what
    // makes it a live test of `push_op`'s drain.
    builder.image(
        shapes.image,
        Affine::scale(70.0, 70.0).then(Affine::translate(30.0, 30.0)),
        0.6,
        ImageFilter::Nearest,
        None,
        BlendMode::Normal,
        None,
    )?;
    let mut order: Vec<u32> = (0..HEAVY_MARKS).collect();
    if reversed {
        order.reverse();
    }
    for index in order {
        fill(
            builder,
            shapes.heavy,
            heavy_at(index),
            f32::from(index as u16) / HEAVY_MARKS as f32,
            None,
        )?;
    }
    Ok(())
}

/// One level of the nesting: marks, then the level below, then more marks — so the walk
/// has queued work on both sides of every plan boundary.
fn level(
    builder: &mut SceneBuilder,
    shapes: &Shapes,
    depth: u32,
    reversed: bool,
) -> Result<(), SceneError> {
    let base = f32::from(depth as u16) * 11.0;
    for step in 0..3_u32 {
        let at = Affine::translate(40.0 + base + f32::from(step as u16) * 5.0, 40.0 + base);
        // One of the three is curve-clipped: the residue path, which stays on the walk's
        // thread while the run around it does not.
        let clip = (step == 1).then_some(shapes.clip);
        fill(
            builder,
            shapes.light,
            at,
            f32::from(step as u16) / 3.0,
            clip,
        )?;
    }
    if depth == 0 {
        heavy_run(builder, shapes, reversed)?;
    } else {
        builder.group(
            GroupSpec {
                alpha: 0.8,
                blend: BlendMode::Normal,
                clip: None,
                knockout: false,
                mask: None,
                isolated: true,
                compose: Compose::SrcOver,
            },
            |body| level(body, shapes, depth - 1, reversed),
        )?;
    }
    for step in 0..3_u32 {
        let at = Affine::translate(120.0 - base - f32::from(step as u16) * 5.0, 120.0 - base);
        fill(
            builder,
            shapes.light,
            at,
            1.0 - f32::from(step as u16) / 3.0,
            None,
        )?;
    }
    Ok(())
}

fn nested_page(device: &mut Device, reversed: bool) -> Scene {
    let heavy = device.upload_outline(&blob(400, 22.0)).unwrap();
    let light = device.upload_outline(&blob(6, 5.0)).unwrap();
    let curve = device.upload_outline(&blob(9, 40.0)).unwrap();
    let image = device
        .upload_image(&ImageSpec {
            width: 2,
            height: 2,
            data: std::sync::Arc::from(
                [
                    255_u8, 240, 0, 255, 0, 200, 255, 255, 255, 0, 160, 255, 40, 255, 90, 255,
                ]
                .as_slice(),
            ),
        })
        .unwrap();
    let mut builder = SceneBuilder::new();
    let shapes = Shapes {
        heavy,
        light,
        image,
        clip: builder
            .clip(
                curve,
                Affine::translate(90.0, 90.0),
                FillRule::NonZero,
                None,
            )
            .expect("a curve clip"),
    };
    level(&mut builder, &shapes, DEPTH, reversed).expect("a valid nested page");
    builder.finish()
}

fn draw(threads: usize, reversed: bool) -> (Vec<u8>, Counters) {
    let mut device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        encode_threads: threads,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs");
    device.wait_until_warm();
    let scene = nested_page(&mut device, reversed);
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

/// **The fixture is order-sensitive**, proved rather than argued.
///
/// The same marks at the same places with the same colours, emitted in the opposite
/// order, must draw a different page. A fixture whose marks do not touch is identical
/// under this transformation, so this is the property `doc/HANDOVER.md` says a determinism
/// fixture must have — and the reason every equality below means something.
#[test]
fn the_nested_fixture_is_order_sensitive() {
    let (forward, counters) = draw(1, false);
    let (backward, _) = draw(1, true);
    assert!(
        forward.iter().skip(3).step_by(4).any(|&a| a > 0),
        "the fixture draws something, or every comparison here is between blank pages"
    );
    assert!(
        counters.tiles > 0 && counters.atlas_distinct_keys > 0,
        "the nested page populates both deferred lanes: {counters:?}"
    );
    assert!(
        forward != backward,
        "the deepest plan's marks compose to the same page in either order, so they do \
         not overlap — ADR 0054's first gate had exactly this shape and passed with an \
         ordering drain removed"
    );
}

/// **A page whose marks are all inside nested groups is the same bytes at every thread
/// count** — §4.6, held where the walk crosses a plan boundary between every run.
#[test]
fn a_nested_page_is_the_same_bytes_at_every_thread_count() {
    let (alone, counters) = draw(1, false);
    for threads in COUNTS.into_iter().skip(1) {
        let (divided, also) = draw(threads, false);
        assert_eq!(
            counters, also,
            "the counters moved between 1 thread and {threads} on a nested page"
        );
        assert!(
            divided == alone,
            "the nested page drawn on {threads} threads is not the page drawn on one"
        );
    }
}

/// The refusal a nested page makes does not move either.
///
/// `tests/encode_threads.rs` holds this for a flat page; a nested one refuses from inside
/// a child plan, where the walk has a plan stack to unwind and a queue that may still hold
/// work. The variant and both of its numbers are the same on any count.
#[test]
fn a_nested_refusal_names_the_same_numbers_at_every_thread_count() {
    let refusal = |threads: usize| {
        let mut device = Device::headless(&Options {
            adapter: Some("llvmpipe".into()),
            encode_threads: threads,
            // Room for the instance streams the walk charges up front and not for the
            // coverage tiles the deepest plan asks for, so the refusal happens inside the
            // nesting rather than before it.
            max_frame_bytes: 60_000,
            ..Options::default()
        })
        .expect("llvmpipe is present wherever this suite runs");
        device.wait_until_warm();
        let scene = nested_page(&mut device, false);
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
            "the nested refusal moved at {threads} threads"
        );
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/<crate> is two levels below the workspace root")
        .to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// **Nothing re-forces the thread count**, which is hayro #1316's actual complaint.
///
/// The count is clamped once — at `Device` construction, against
/// [`std::thread::available_parallelism`] — and read from there for the frame. No plan
/// boundary, no child, no soft mask and no nested body may lower it, so a host that asked
/// for threads gets them wherever its marks are.
///
/// A source gate rather than a behavioural one because the division is unobservable
/// through the public API on purpose: `Counters` must be identical at every thread count
/// for `tests/encode_threads.rs` to mean what it says, so a counter that recorded the
/// division would contradict the design it would be measuring. What a reassignment would
/// look like is `self.threads = 1;` inside `plan_child`, and this is what would see it.
#[test]
fn the_thread_count_is_never_reassigned_after_the_frame_has_it() {
    let mut sources = Vec::new();
    rust_files(
        &workspace_root().join("crates/quorra-gpu/src"),
        &mut sources,
    );
    assert!(
        sources.len() > 30,
        "the walk found only {} source files, so it is not walking the crate",
        sources.len()
    );
    for path in &sources {
        let text = std::fs::read_to_string(path).expect("a source file");
        for assignment in [
            "self.threads =",
            "self.threads=",
            "self.encode_threads =",
            "self.encode_threads=",
        ] {
            assert!(
                !text.contains(assignment),
                "{} assigns the thread count after the frame has it (`{assignment}`). \
                 hayro #1316 is a renderer that re-forces its thread count for nested \
                 contexts, so an embedder cannot opt in at all; `Options::encode_threads` \
                 is clamped once at construction and read from there, and a second write \
                 is how that property is lost.",
                path.display()
            );
        }
    }
}

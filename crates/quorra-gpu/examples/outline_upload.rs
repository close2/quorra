//! What an outline upload costs, and where the GPU lane's conversion went (ADR 0075).
//!
//! ```text
//! cargo run --release -p quorra-gpu --example outline_upload [-- <adapter substring> [rounds]]
//! ```
//!
//! # The number this exists to hold
//!
//! `Device::upload_outline` used to validate an outline, recognise its rectangle, **and
//! convert it into the quadratic contours the GPU coverage lane draws** — a conversion
//! read in exactly one place, behind a setting whose default answers `false` on sight.
//! The caller measured what that cost on the project owner's own 3 011 919-segment
//! drawing (`QUORRA_FEEDBACK.md` §33): of a 187.6 ms first-frame scene phase, **156.0 ms
//! — 83 % — was inside our `upload_*` calls**, and six sevenths of that was
//! `QuadOutline::from_segments`.
//!
//! That is their number. This is ours, and it is measured on this side of the boundary
//! so that a regression here fails here.
//!
//! # Four parts, and why each is the kind of statement it is
//!
//! - **§A, what an upload costs**, by the shape of the outline uploaded. Two arms
//!   round-robin — an all-cubic corpus and an all-straight one of the same segment count
//!   — because the cost that moved is *subdivision*, and an arm that does none is the
//!   control that says so. This is the arm to run against a tree without ADR 0075 in it:
//!   the binary compiles against both, so the comparison is one instrument and two
//!   libraries rather than two instruments.
//! - **§B, where the conversion went.** Three arms round-robin, each on a device of its
//!   own because an outline converts once and a warm one cannot be re-measured: the first
//!   frame under `Coverage::Cpu`, the first under `Coverage::Gpu`, and the second under
//!   `Coverage::Gpu`. The corpus is chosen so that **every mark draws identically in all
//!   three** — its outlines pass `gpu_lane_admissible` and are then declined on triangle
//!   count (ADR 0026), so the only difference between the arms is the conversion. Three
//!   numbers are printed and their difference is not: a subtraction of wall clocks is a
//!   claim, and the two arms that convert nothing are what makes it checkable.
//!
//!   **The column that carries §B is the byte column, not the clock.** A frame's
//!   conversion is a fraction of a frame that also flattens every one of those segments
//!   on the processor, and the first frame of a device pays for pipelines and buffer
//!   growth besides — ADR 0075 measured the same first-against-second gap on a tree that
//!   converts at upload and has nothing to defer, which is the control that says the gap
//!   is not the conversion. The bytes have no such problem: the first `Coverage::Gpu`
//!   frame converts and every other frame in the table converts nothing, on every run.
//! - **§C, the witness**, which is not a clock at all and is half of `--check`.
//!   `Device::resource_bytes_in_use` counts what is resident, so it *rises when the
//!   conversion happens* and at no other time. Every claim §A and §B make about *when*
//!   an outline converts is asserted there, deterministically, on a corpus small enough
//!   for CI.
//! - **§D, the page the device lane actually draws**, reduced to a digest. §C's marks are
//!   declined on triangle count, which is what makes its arms one picture and also means
//!   it never reads a converted outline's triangles. §D does, and prints a number two
//!   builds of this binary can be compared by — which is how ADR 0075's "it moves no
//!   pixel" was checked between the trees rather than argued from the code.
//!
//! # What the printed clock can and cannot say
//!
//! Minima of round-robin rounds, never means, with the load average printed either side
//! of every sample — this machine is somebody's desktop, and a reader who does not like
//! the load discounts the run rather than the conclusion. Round 0 is reported apart and
//! never enters a minimum: the first upload of a process pays the allocator's first
//! growth of every arena the corpus touches.

// A measurement binary's arithmetic is counts and indices over a corpus it built itself;
// the library's own lints are stricter because its inputs come from a document.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use std::time::{Duration, Instant};

use quorra_gpu::{Coverage, Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, OutlineId, Paint, Point, Scene, SceneBuilder,
    Segment,
};

/// A small atlas, so §B's and §C's marks are tiles the cache will not hold and
/// `gpu_lane_admissible` therefore reaches its last test (ADR 0024's eighth of 64 KiB is
/// 8 KiB, and every tile below is larger). Without it the whole file would measure the
/// atlas lane and convert nothing, passing while testing nothing.
const TINY_ATLAS: u64 = 64 * 1024;

/// §A's corpus: outlines, and segments in each. 2 000 × 200 is 400 000 segments, which
/// is an eighth of the caller's document and enough that the arms separate by more than
/// the round-to-round spread on a loaded desktop.
const CORPUS_OUTLINES: usize = 2_000;
const CORPUS_SEGMENTS: usize = 200;

/// §B's corpus. Each outline is a wiggle of [`MARK_SEGMENTS`] cubics inside a
/// [`MARK_SIDE`]-pixel box: large enough that the atlas declines it, and *dense* enough
/// that the triangle test then declines it too, so all three arms rasterise the same way
/// on the processor and differ only in whether they converted.
const MARKS: usize = 48;
const MARK_SEGMENTS: usize = 120;
const MARK_SIDE: f32 = 128.0;

/// §C's corpus, and `--check`'s: small enough to run in CI, large enough that a
/// conversion's bytes are unmistakable next to the segments'.
const WITNESS_OUTLINES: usize = 8;
const WITNESS_SEGMENTS: usize = 32;

/// A closed wiggle of `segments` cubic curves around a circle of radius `radius`,
/// centred at `centre`, with `seed` moving every control point off the last one's.
///
/// Cubics, and cubics that genuinely bend: the conversion ADR 0075 deferred subdivides
/// until Loop and Blinn's third-difference bound holds, so a corpus of straight-ish
/// curves would measure the bound's early exit rather than the subdivision.
fn wiggle(segments: usize, centre: (f32, f32), radius: f32, seed: usize) -> Vec<Segment> {
    let at = |i: usize| {
        let turn = i as f32 / segments as f32 * std::f32::consts::TAU;
        // A radius that varies with the index makes each span a curve with a real
        // third difference rather than an arc a single quadratic already fits.
        let r = radius * (0.55 + 0.45 * ((i * 7 + seed) % 11) as f32 / 11.0);
        Point::new(centre.0 + r * turn.cos(), centre.1 + r * turn.sin())
    };
    let mut path = Vec::with_capacity(segments + 2);
    path.push(Segment::MoveTo(at(0)));
    for i in 0..segments {
        path.push(Segment::CubicTo {
            c1: at(i * 3 + 1),
            c2: at(i * 3 + 2),
            to: at((i + 1) % segments),
        });
    }
    path.push(Segment::Close);
    path
}

/// The same corpus with every curve replaced by the chord it spans: the control arm.
///
/// One `LineTo` per `CubicTo`, so the segment count and the validation walk are the
/// same and the only thing missing is the subdivision.
fn chords(path: &[Segment]) -> Vec<Segment> {
    path.iter()
        .map(|segment| match *segment {
            Segment::CubicTo { to, .. } => Segment::LineTo(to),
            other => other,
        })
        .collect()
}

/// §A's corpus: `count` distinct outlines of `segments` cubics each, built before any
/// clock starts.
fn corpus(count: usize, segments: usize) -> Vec<Vec<Segment>> {
    (0..count)
        .map(|i| wiggle(segments, (60.0, 60.0), 50.0, i))
        .collect()
}

/// One §A sample: upload every outline of a corpus, timing that and nothing else, then
/// release them so the next sample starts from the same resident bytes.
///
/// The release is outside the span deliberately — it is not what §33 asked about — and
/// so is the corpus's own construction, which happens once for the whole run.
fn upload_sample(device: &mut Device, corpus: &[Vec<Segment>]) -> Duration {
    let mut ids = Vec::with_capacity(corpus.len());
    let started = Instant::now();
    for path in corpus {
        ids.push(device.upload_outline(path).expect("within the budget"));
    }
    let elapsed = started.elapsed();
    for id in ids {
        device.release(id).expect("what was uploaded is resident");
    }
    elapsed
}

/// A page of `MARKS` marks, one per outline, laid out in a row of boxes.
fn page(outlines: &[OutlineId]) -> Scene {
    let mut builder = SceneBuilder::new();
    for (i, outline) in outlines.iter().enumerate() {
        builder
            .fill(
                *outline,
                Affine::translate((i % 8) as f32 * MARK_SIDE, (i / 8) as f32 * MARK_SIDE),
                FillRule::NonZero,
                Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0)),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .expect("a solid fill of a resident outline");
    }
    builder.finish()
}

/// The viewport §B and §C draw through: whole boxes, at 1× so the marks keep the size
/// their outlines state.
fn viewport(marks: usize) -> Viewport<'static> {
    let columns = 8u32.min(marks as u32).max(1);
    let rows = (marks as u32).div_ceil(columns);
    Viewport::full(
        columns * MARK_SIDE as u32,
        rows * MARK_SIDE as u32,
        Affine::IDENTITY,
    )
}

/// A device with a cache too small to stand in front of these marks — and with the
/// per-frame hybrid pinned off, because this file's whole construction is that the
/// atlas-declined marks rasterise on the processor in every arm. ADR 0090's auto rule
/// would reroute them to the compute lane on any hardware adapter, and the compute
/// lane's picture agrees with the scanline's only to one coverage step (ADR 0094) —
/// which would make §B time two pictures and fail §C's identity for a reason this
/// file does not measure. The hybrid has its own suite (`tests/compute_assist.rs`).
fn device_for(adapter: Option<String>, coverage: Coverage) -> Device {
    let device = Device::headless(&Options {
        adapter,
        coverage,
        atlas_budget: TINY_ATLAS,
        compute_assist: Some(false),
        ..Options::default()
    })
    .expect("an adapter");
    // A background warm-up thread still compiling would be contending with every span
    // below, and the pipelines it builds are not what this file measures.
    device.wait_until_warm();
    device
}

/// One §B sample: a fresh device, a corpus uploaded off the clock, and `warm_up` frames
/// drawn off it, so that the span covers exactly the frame under test.
fn frame_sample(
    adapter: Option<String>,
    coverage: Coverage,
    marks: usize,
    segments: usize,
    warm_up: usize,
) -> (Duration, u64) {
    let mut device = device_for(adapter, coverage);
    let outlines: Vec<OutlineId> = (0..marks)
        .map(|i| {
            let path = wiggle(
                segments,
                (MARK_SIDE / 2.0, MARK_SIDE / 2.0),
                MARK_SIDE / 2.2,
                i,
            );
            device.upload_outline(&path).expect("within the budget")
        })
        .collect();
    let scene = page(&outlines);
    let viewport = viewport(marks);
    for _ in 0..warm_up {
        device
            .render(&scene, &viewport, Target::Readback)
            .expect("the page is inside every budget");
    }
    let uploaded = device.resource_bytes_in_use();
    let started = Instant::now();
    let frame = device
        .render(&scene, &viewport, Target::Readback)
        .expect("the page is inside every budget");
    let elapsed = started.elapsed();
    drop(frame);
    (elapsed, device.resource_bytes_in_use() - uploaded)
}

fn load_average() -> String {
    std::fs::read_to_string("/proc/loadavg").map_or_else(
        |_| "unknown".into(),
        |line| {
            line.split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ")
        },
    )
}

/// §D: **the page the GPU lane actually draws, reduced to a number two builds can be
/// compared by.**
///
/// §C's marks are declined on triangle count, which is what makes its three arms one
/// picture — and it means §C never exercises the code path that reads a converted
/// outline's triangles. This does: one two-cubic blob over a box the atlas will not
/// cache, which `triangles_under_coverage` admits because sixteen triangles are cheaper
/// than a quarter of a million coverage bytes.
///
/// The assertion is the control — the two lanes must *disagree* on a curved edge, or the
/// device lane was not taken and the digest would be a statement about the processor.
/// The digest itself asserts nothing here, because a raster is a property of the adapter:
/// it is printed so that this binary built against two trees can be held to the same
/// pixels, which is how ADR 0075's "it moves no pixel" was checked rather than argued.
///
/// FNV-1a over the straight-alpha RGBA §3 hands back, because the deliverable is a
/// comparison between two runs and not a hash anybody stores.
fn gpu_lane_digest(adapter: Option<String>) -> u64 {
    let mut device = device_for(adapter, Coverage::Cpu);
    let side = 4.0 * MARK_SIDE;
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(side * 0.15, side * 0.15)),
            Segment::CubicTo {
                c1: Point::new(side * 0.85, side * 0.05),
                c2: Point::new(side * 0.95, side * 0.65),
                to: Point::new(side * 0.50, side * 0.85),
            },
            Segment::CubicTo {
                c1: Point::new(side * 0.20, side * 0.95),
                c2: Point::new(side * 0.05, side * 0.50),
                to: Point::new(side * 0.15, side * 0.15),
            },
            Segment::Close,
        ])
        .expect("within the budget");
    let scene = page(&[outline]);
    let view = Viewport::full(side as u32, side as u32, Affine::IDENTITY);
    let raster = |device: &mut Device| {
        device
            .render(&scene, &view, Target::Readback)
            .expect("one blob is inside every budget")
            .into_raster()
            .expect("a Readback frame carries a raster")
            .into_pixels()
    };
    let cpu = raster(&mut device);
    device.set_coverage(Coverage::Gpu);
    let gpu = raster(&mut device);
    assert_ne!(
        cpu, gpu,
        "the two lanes answer a curved edge differently, so equality here means the \
         device lane was never taken and the digest below says nothing about it"
    );
    gpu.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// §C: the assertions, which are about *when* an outline converts and not about how
/// long it takes. Every one of them is a statement about
/// [`Device::resource_bytes_in_use`], which counts what is resident and therefore rises
/// exactly when the conversion happens.
fn witness(adapter: Option<String>) {
    let mut device = device_for(adapter, Coverage::Cpu);
    let outlines: Vec<OutlineId> = (0..WITNESS_OUTLINES)
        .map(|i| {
            let path = wiggle(
                WITNESS_SEGMENTS,
                (MARK_SIDE / 2.0, MARK_SIDE / 2.0),
                MARK_SIDE / 2.2,
                i,
            );
            device.upload_outline(&path).expect("within the budget")
        })
        .collect();

    // The upload charges the segments and only the segments. `Segment` is the scene's
    // own enum, so the arithmetic is the store's, restated here from the outside.
    let segment_bytes = (WITNESS_OUTLINES * (WITNESS_SEGMENTS + 2) * size_of::<Segment>()) as u64;
    let after_upload = device.resource_bytes_in_use();
    assert_eq!(
        after_upload, segment_bytes,
        "an upload must charge the segments it stored and nothing it has not built"
    );

    let scene = page(&outlines);
    let view = viewport(WITNESS_OUTLINES);
    let cpu = device
        .render(&scene, &view, Target::Readback)
        .expect("the page is inside every budget")
        .into_raster()
        .expect("a Readback frame carries a raster")
        .into_pixels();
    assert_eq!(
        device.resource_bytes_in_use(),
        after_upload,
        "a frame on the processor lane must convert nothing: that is the whole of ADR 0075"
    );

    device.set_coverage(Coverage::Gpu);
    let gpu = device
        .render(&scene, &view, Target::Readback)
        .expect("the page is inside every budget")
        .into_raster()
        .expect("a Readback frame carries a raster")
        .into_pixels();
    let after_conversion = device.resource_bytes_in_use();
    assert!(
        after_conversion > after_upload,
        "the first frame that asks for the GPU lane's geometry must build it and be \
         charged for it: {after_conversion} is not more than {after_upload}"
    );

    device
        .render(&scene, &view, Target::Readback)
        .expect("the page is inside every budget");
    assert_eq!(
        device.resource_bytes_in_use(),
        after_conversion,
        "and a second frame must re-read that conversion rather than making it again"
    );

    // The corpus is chosen so the lane is *reached* and then declined on triangle count
    // (ADR 0026), which is what makes §B's three arms comparable: same pixels, and only
    // the conversion between them.
    assert_eq!(
        cpu, gpu,
        "these marks are declined by the triangle test under either setting, so the two \
         rasters are one page — a difference here means §B is timing two pictures"
    );

    for id in outlines {
        device.release(id).expect("what was uploaded is resident");
    }
    assert_eq!(
        device.resource_bytes_in_use(),
        0,
        "a release must return the conversion's charge as well as the upload's"
    );
    println!(
        "§C  upload charged {after_upload} B, the first GPU-lane frame charged \
         {} B more, the second charged nothing, and a release returned both",
        after_conversion - after_upload
    );
}

/// §A's two arms, in the order printed.
const SHAPES: [&str; 2] = ["cubic", "chord"];

/// `--check`: §C alone, on the small corpus — the smallest run that executes every
/// assertion this file makes. §A and §B print statistics and assert nothing, because a
/// one-round wall clock is not a measurement and must not read like one (ADR 0060).
fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let check = args.iter().any(|arg| arg == "--check");
    args.retain(|arg| arg != "--check");
    let mut args = args.into_iter();
    let adapter = args.next();
    let rounds: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(9);

    let mut device = device_for(adapter.clone(), Coverage::Cpu);
    println!("adapter: {}", device.description());
    if check {
        let digest = gpu_lane_digest(adapter.clone());
        println!("§D  the GPU lane's page digests to {digest:#018x}");
        witness(adapter);
        return;
    }

    let cubic = corpus(CORPUS_OUTLINES, CORPUS_SEGMENTS);
    let chord: Vec<Vec<Segment>> = cubic.iter().map(|path| chords(path)).collect();
    let segments: usize = cubic.iter().map(Vec::len).sum();
    println!(
        "\n§A  {CORPUS_OUTLINES} outlines of {CORPUS_SEGMENTS} segments — {segments} \
         segments per sample"
    );
    let mut best = [None::<Duration>; SHAPES.len()];
    for round in 0..=rounds {
        let apart = if round == 0 {
            " — reported apart: the first upload of a process pays the allocator's first growth"
        } else {
            ""
        };
        println!("round {round}{apart}");
        for (at, arm) in [&cubic, &chord].into_iter().enumerate() {
            let before = load_average();
            let elapsed = upload_sample(&mut device, arm);
            let after = load_average();
            println!(
                "  {:<6}  {elapsed:>12?}  {:>6.1} ns/segment  load {before} before / {after} after",
                SHAPES[at],
                elapsed.as_nanos() as f64 / segments as f64
            );
            if round > 0 {
                let slot = &mut best[at];
                *slot = Some(slot.map_or(elapsed, |slot| slot.min(elapsed)));
            }
        }
    }
    println!("\n§A minima over {rounds} round-robin rounds, first round excluded:");
    for (at, shape) in SHAPES.iter().enumerate() {
        match best[at] {
            Some(best) => println!(
                "  {shape:<6}  {best:>12?}  {:>6.1} ns/segment",
                best.as_nanos() as f64 / segments as f64
            ),
            None => println!("  {shape:<6}  no sample"),
        }
    }

    let arms: [(&str, Coverage, usize); 3] = [
        ("first, Cpu", Coverage::Cpu, 0),
        ("first, Gpu", Coverage::Gpu, 0),
        ("second, Gpu", Coverage::Gpu, 1),
    ];
    println!(
        "\n§B  {MARKS} marks of {MARK_SEGMENTS} segments, one outline each — \
         {} segments, drawn identically by all three arms",
        MARKS * (MARK_SEGMENTS + 2)
    );
    let mut frames = [None::<Duration>; 3];
    for round in 0..=rounds {
        println!("round {round}{}", if round == 0 { " — apart" } else { "" });
        for (at, (name, coverage, warm_up)) in arms.into_iter().enumerate() {
            let before = load_average();
            let (elapsed, converted) =
                frame_sample(adapter.clone(), coverage, MARKS, MARK_SEGMENTS, warm_up);
            let after = load_average();
            println!(
                "  {name:<12}  {elapsed:>12?}  {converted:>9} B converted in it  \
                 load {before} before / {after} after"
            );
            if round > 0 {
                let slot = &mut frames[at];
                *slot = Some(slot.map_or(elapsed, |slot| slot.min(elapsed)));
            }
        }
    }
    println!("\n§B minima over {rounds} round-robin rounds, first round excluded:");
    for (at, (name, _, _)) in arms.into_iter().enumerate() {
        match frames[at] {
            Some(best) => println!("  {name:<12}  {best:>12?}"),
            None => println!("  {name:<12}  no sample"),
        }
    }
    // Last, not first: §C is what fails on a tree that still converts at upload, and a
    // run against such a tree is exactly the comparison §A exists to make. Failing after
    // the numbers are printed is what makes that run useful rather than empty. §D goes
    // ahead of it for the same reason — its digest is the thing to compare between the
    // two trees, so it must be printed before the assertion that one of them fails.
    let digest = gpu_lane_digest(adapter.clone());
    println!("\n§D  the GPU lane's page digests to {digest:#018x}");
    witness(adapter);
    println!("\nload average at the end: {}", load_average());
}

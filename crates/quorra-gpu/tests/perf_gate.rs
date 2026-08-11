//! The M1 performance gate: a real page's shape at a real window's scale.
//!
//! CLAUDE.md principle 2: perf gates run in CI with numbers attached, and where a
//! gate needs to be deterministic it uses timestamp queries rather than a stopwatch —
//! wall clocks lie under load, and CI runners are always under load. So this gate
//! reads `Timings::execute` from timestamp queries and skips (loudly) when the
//! adapter has none.
//!
//! The thresholds are set from the measured M1 numbers in `doc/PLAN.md` (fastest of
//! ten, release build, 2026-08-02): encode 0.035 ms on either adapter; execute
//! 0.048 ms on RADV and 2.6 ms on llvmpipe for this scene at this scale. The gates
//! are ~10-20x those values — wide enough for a loaded CI runner and a debug-built
//! encode, tight enough that a lane falling off its fast path fails the build.

// The f64→f32 casts build scene coordinates bounded by the page size; exact there.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation
)]

use std::time::Duration;

use quorra_gpu::{Coverage, Device, Options, Target, TimingProvenance, Viewport};
use quorra_scene::{Affine, Color, Point, Rect, Scene, SceneBuilder};

/// A dense page's shape: thousands of small rectangles. (5 933 is one dense page's
/// glyph count in the brief; rectangles stand in for glyphs until M4.)
fn dense_scene() -> Scene {
    let mut builder = SceneBuilder::new();
    for i in 0..5_933_u32 {
        let x = f64::from(i % 80).mul_add(14.5, 3.25) as f32;
        let y = f64::from(i / 80).mul_add(15.25, 4.5) as f32;
        builder
            .rect(
                Rect::new(Point::new(x, y), Point::new(x + 9.75, y + 11.5)),
                Affine::IDENTITY,
                Color::new(0.1, 0.1, 0.1, 1.0),
                None,
                None,
            )
            .unwrap();
    }
    builder.finish()
}

#[test]
fn dense_page_at_window_scale_stays_under_the_gate() {
    let mut device = Device::headless(&Options::default()).expect("some adapter must exist");
    device.wait_until_warm();
    let scene = dense_scene();
    let viewport = Viewport::full(1191, 1684, Affine::IDENTITY);

    // Fastest of five: the project's own convention for wall-adjacent numbers.
    let mut best_execute = Duration::MAX;
    let mut best_encode = Duration::MAX;
    let mut provenance = TimingProvenance::WallClock;
    for _ in 0..5 {
        let frame = device
            .render(&scene, &viewport, Target::Readback)
            .expect("the dense page is within every budget");
        let timings = frame.timings();
        best_execute = best_execute.min(timings.execute);
        best_encode = best_encode.min(timings.encode);
        provenance = timings.execute_provenance;
    }
    eprintln!(
        "perf gate on {}: encode {best_encode:?}, execute {best_execute:?} ({provenance:?})",
        device.description()
    );

    // Encode is CPU-side and wall-clocked by nature; its gate is generous for that.
    assert!(
        best_encode < Duration::from_millis(10),
        "encoding 5 933 rects took {best_encode:?}; measured 0.035 ms in release (PLAN.md M1)"
    );
    match provenance {
        TimingProvenance::TimestampQueries => {
            assert!(
                best_execute < Duration::from_millis(50),
                "device execution took {best_execute:?}; measured 0.048 ms (RADV) / 2.6 ms \
                 (llvmpipe) for this scene at this scale (PLAN.md M1)"
            );
        }
        TimingProvenance::WallClock => {
            eprintln!("note: no timestamp queries on this adapter; the execute gate did not run");
        }
    }
}

/// The readback gate: tier 1's price, which is most of an offscreen frame (§6.1).
///
/// A `Readback` frame at page size pays a copy-out, a map and the premultiplied→straight
/// conversion over 8 MB, and nothing else in the frame is close to it — the same page to
/// a `Texture` target is 0.29 ms end to end while this one is 1.65 ms. That makes it the
/// number the caller's offscreen corpus gate is mostly measuring, so it gets a gate of
/// its own.
///
/// Wall-clocked by nature (the span is CPU-bound and includes a wait), so the threshold
/// is ~4× the measured value rather than tight: RADV, release, fastest of five,
/// 2026-08-11 — readback 1.32 ms, whole frame 1.65 ms (ADR 0022; it was 3.84 and 4.94
/// before). A regression to the shape that existed before this ADR fails here.
#[test]
fn a_readback_frame_does_not_pay_for_its_pixels_twice() {
    let mut device = Device::headless(&Options::default()).expect("some adapter must exist");
    device.wait_until_warm();
    let scene = dense_scene();
    let viewport = Viewport::full(1191, 1684, Affine::IDENTITY);

    let mut best = Duration::MAX;
    for _ in 0..5 {
        let frame = device
            .render(&scene, &viewport, Target::Readback)
            .expect("the dense page is within every budget");
        best = best.min(frame.timings().readback);
        // The raster is what the readback produced; dropping it unread would let a
        // future implementation defer the work this gate is timing.
        let raster = frame
            .into_raster()
            .expect("a Readback frame carries its pixels");
        assert_eq!(raster.pixels().len(), 1191 * 1684 * 4);
    }
    eprintln!(
        "readback gate on {}: {best:?} for 1191x1684",
        device.description()
    );
    // The conversion is a byte loop, so the two builds are 26× apart — 1.32 ms in
    // release against 34.9 ms in debug, both measured. One threshold could only be
    // generous enough for debug and therefore blind in release, so each build gets the
    // gate its own number earns, at ~2-4× it.
    let (limit, measured) = if cfg!(debug_assertions) {
        (Duration::from_millis(80), "34.9 ms in debug")
    } else {
        (Duration::from_millis(6), "1.32 ms in release")
    };
    assert!(
        best < limit,
        "reading back a page took {best:?}; measured {measured} on RADV (ADR 0022), \
         against 3.84 ms in release before it"
    );
}

/// The zoom gate, and it counts rather than times (ADR 0015).
///
/// A viewer at 20× hands over a whole page for a window showing a fortieth of it; the
/// encoder must reject what cannot reach the target instead of flattening it. The
/// assertion is on `commands_culled`, which is a deterministic function of the scene
/// and the viewport — no wall clock, so a loaded CI runner cannot make it flake, and
/// a lane that stops culling fails the build even on a machine too slow to time.
///
/// The count is exact rather than approximate, and it is arithmetic rather than an
/// observation: at 20× the window spans 59.55 × 84.2 page units about (580, 565), so
/// it holds rectangle columns 38–41 and rows 34–39 — 24 of the 5 933 — and 5 909 are
/// outside by far more than the two device pixels the encoder inflates a command's
/// bounds by (0.1 page units here). The number moves only if the scene or the
/// viewport moves.
#[test]
fn a_zoomed_page_culls_what_it_cannot_reach() {
    let mut device = Device::headless(&Options::default()).expect("some adapter must exist");
    device.wait_until_warm();
    let scene = dense_scene();

    let whole_page = Viewport::full(1191, 1684, Affine::IDENTITY);
    let drawn = device
        .render(&scene, &whole_page, Target::Readback)
        .expect("the dense page is within every budget");
    assert_eq!(
        drawn.counters().commands_culled,
        0,
        "at 1x the whole page is on the target, so nothing may be culled"
    );

    // 20x about the page's middle, as `examples/zoom.rs` measures it.
    let zoomed = Affine::translate(-580.0, -565.0)
        .then(Affine::scale(20.0, 20.0))
        .then(Affine::translate(1191.0 / 2.0, 1684.0 / 2.0));
    let drawn = device
        .render(
            &scene,
            &Viewport::full(1191, 1684, zoomed),
            Target::Readback,
        )
        .expect("the dense page is within every budget");
    assert_eq!(
        drawn.counters().commands_culled,
        5_909,
        "at 20x, 24 of the 5 933 rectangles reach the window and the rest must not be built"
    );
}

/// The GPU coverage lane draws the same page, and culls the same commands.
///
/// A counting gate, like the zoom one above and for the same reason: which lane made
/// the bytes is a cost decision, and the *number of commands a frame builds* is not
/// allowed to depend on it. A lane that quietly dropped work would show here as a
/// changed count long before anyone noticed a thinner page.
///
/// What it deliberately does not gate is time. The two lanes cross over somewhere
/// between 4× and 20× on this machine (ADR 0016's table), the crossing moves with the
/// adapter, and a threshold either side of it would be a number about this laptop.
#[test]
fn the_gpu_lane_draws_and_culls_the_same_frame() {
    let mut device = Device::headless(&Options {
        coverage: Coverage::Gpu,
        ..Options::default()
    })
    .expect("some adapter must exist");
    device.wait_until_warm();
    let scene = dense_scene();

    let drawn = device
        .render(
            &scene,
            &Viewport::full(1191, 1684, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the dense page is within every budget");
    assert_eq!(drawn.counters().commands_culled, 0);
    assert_eq!(drawn.counters().commands, 5_933);

    let zoomed = Affine::translate(-580.0, -565.0)
        .then(Affine::scale(20.0, 20.0))
        .then(Affine::translate(1191.0 / 2.0, 1684.0 / 2.0));
    let drawn = device
        .render(
            &scene,
            &Viewport::full(1191, 1684, zoomed),
            Target::Readback,
        )
        .expect("the dense page is within every budget");
    assert_eq!(
        drawn.counters().commands_culled,
        5_909,
        "the cull is the encoder's, so both lanes reject exactly the same commands"
    );
}

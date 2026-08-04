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

use quorra_gpu::{Device, Options, Target, TimingProvenance, Viewport};
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

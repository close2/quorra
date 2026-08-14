//! What a `Readback` frame costs in host memory — ADR 0022's claim, as a property.
//!
//! Tier 1's price is the largest single item in an offscreen frame (§6.1), and ADR 0022
//! is the decision that made it *"read once and divide never"*: the demultiply runs
//! straight out of the mapped range, where the shape before it staged the whole target
//! into a `Vec` first and then converted that. The saving is a full target buffer —
//! 8 MB at page size — of allocation, copy and traffic.
//!
//! **Why this is an allocation count and not a stopwatch** (ADR 0052). The gate that
//! guarded this used to be a wall clock, and it could not do the job twice over: its
//! threshold was 6 ms against a regression whose measured value is 3.84 ms, so the
//! regression it names would have passed it, while ambient load on the development
//! machine failed it two runs in five. Both halves are the same mistake — CLAUDE.md
//! principle 2 says to count rather than to time where a gate must be deterministic, and
//! `HANDOVER.md` says wall clocks on this machine are worthless at the load averages it
//! runs at.
//!
//! What is left un-gated by the change is honest to state: the *divide* half of ADR 0022
//! is a throughput claim, and no wall clock here can hold it. Its instrument is
//! callgrind, and `HANDOVER.md`'s "An encode, exactly" describes the harness.

// Test-crate `expect`s are how a fixture states a precondition it cannot proceed without.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation
)]

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{Affine, Color, Point, Rect, Scene, SceneBuilder};

mod counting_allocator;

use counting_allocator::Watch;

/// The page this gate reads back: the perf gate's own dense fixture, at the caller's
/// window size, so the number below is a real page's target and not a round one.
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

/// A `Readback` frame allocates exactly one target-sized buffer: the raster it returns.
///
/// The assertion is an equality rather than a bound, and deliberately: the pre-ADR 0022
/// shape allocates a second buffer of the copy-out's *padded* extent — 8 191 kB against
/// the raster's 8 023 — so a bound stated in megabytes would have to be loose enough to
/// admit it. Measured both ways before this test was written: healthy, one allocation of
/// 8 022 576 bytes; with `read_buffer`'s staging `Vec` put back, two totalling
/// 16 213 552. Nine runs of each at load averages from 7 to 30, identical every time.
#[test]
fn a_readback_frame_allocates_one_target_and_no_second_copy() {
    // The default adapter, deliberately: this is a property of the host code and holds
    // on either of this machine's two. Verified on both when it was written — RADV and
    // llvmpipe (LLVM 22.1.8) each report one allocation of 8 022 576 bytes.
    let mut device = Device::headless(&Options::default()).expect("some adapter must exist");
    device.wait_until_warm();
    let scene = dense_scene();
    let viewport = Viewport::full(1191, 1684, Affine::IDENTITY);

    // Three frames first, so that what the watched frame allocates is the frame's rather
    // than a pool reaching its steady size. Every pool here grows and never shrinks
    // (PLAN.md §1.5), so three is enough and the fourth would prove nothing.
    for _ in 0..3 {
        let frame = device
            .render(&scene, &viewport, Target::Readback)
            .expect("the dense page is within every budget");
        drop(
            frame
                .into_raster()
                .expect("a Readback frame carries its pixels"),
        );
    }

    let watch = Watch::start();
    let frame = device
        .render(&scene, &viewport, Target::Readback)
        .expect("the dense page is within every budget");
    let raster = frame
        .into_raster()
        .expect("a Readback frame carries its pixels");
    let (allocations, bytes) = watch.finish();

    eprintln!(
        "readback cost on {}: {allocations} target-sized allocation(s), {bytes} bytes, \
         for a {}-byte raster",
        device.description(),
        raster.pixels().len(),
    );

    assert_eq!(
        raster.pixels().len(),
        1191 * 1684 * 4,
        "the raster is the target's pixels in straight alpha (§3)"
    );
    assert_eq!(
        allocations, 1,
        "a readback frame allocated {allocations} target-sized buffers; ADR 0022 leaves \
         exactly one, the raster itself. Two is the staging `Vec` this device stopped \
         paying for — see `readback::map_and_convert`"
    );
    assert_eq!(
        bytes,
        raster.pixels().len(),
        "the one target-sized allocation is not the raster's own {} bytes; something \
         else of target size is being built inside the frame",
        raster.pixels().len()
    );
}

//! The one count in this tree that is chunked to a fixed width, asked for at a size that
//! is deliberately **not** a multiple of it.
//!
//! `doc/notes-ceilings-audit.md` §3 is the round this file witnesses, and the reason it
//! exists is the caller's, from `pdf-viewer/doc/HAYRO_ISSUES_FOR_QUORRA.md` §1 on hayro's
//! `#373` — a SIMD flattening path that read past its own scratch buffer:
//!
//! > It is the failure mode of a lane-width-rounded buffer whose tail is not padded, and
//! > it is invisible to every test whose path length happens to be a multiple of the lane
//! > width. Worth a look at any place quorra rounds a segment count up to a vector width.
//!
//! **This tree rounds nothing up to a vector width**, and the audit says why with the
//! search behind it. What it does have is one count chunked to a fixed width: the GPU
//! coverage lane's sample grid is written four samples at a time, because four fit one
//! `rgba16float` texel and the winding pass runs once per group of four
//! (`winding/buffers.rs`, `SAMPLES_PER_PASS`). The count itself is rounded **down** to a
//! perfect square at construction — `{4, 9, 16, 25, 36, 49, 64}` — so three of the seven
//! reachable values leave a last group holding **one** sample rather than four.
//!
//! Nine samples is that case, and it is the whole of this file. Sixteen is what every
//! other test in the suite uses, and sixteen is a multiple of four: the tail this file
//! exercises is exactly the one `#373` says every existing test misses.
//!
//! # What must hold, and why it is derivable
//!
//! The tail is not padded on the host. It is answered on the device, in two halves that a
//! reader has to hold together — which is why the property is stated here rather than left
//! to the comments:
//!
//! - `winding/passes.rs` clears the winding attachment at the start of every round, so a
//!   channel the short group never wrote holds winding 0, and `winding.wgsl`'s `inside(0,
//!   rule)` is false under both of ISO 32000-2 §8.5.3.3's rules — so an unwritten channel
//!   adds nothing to the sum;
//! - the divisor `winding.wgsl` divides that sum by is the **total** sample count, not the
//!   number of groups times four, so the fraction is exact.
//!
//! Get either half wrong and a wholly covered pixel falls far short: with nine samples in
//! three groups, a divisor of twelve reads 191 and a fourth phantom channel per group
//! reads the same. A wholly covered pixel is the sharpest probe there is, because its
//! answer does not depend on the sample positions at all.
//!
//! # What a short tail *does* cost, measured
//!
//! Each round deposits its own share into the frame's R8 coverage sheet, so each share is
//! rounded to a byte before the next is added — the sum of `g` rounded shares differs from
//! the exact answer by at most `g/2` steps, and saturation at 255 recovers the loss only
//! where the shares round up. Measured on llvmpipe, a wholly covered pixel reads **255 at
//! 4, 16, 25 and 49 samples and 254 at 9**: `4/9` rounds down twice (113, 113) and the
//! tail's `1/9` rounds down again (28), where at 25 and 49 the six or twelve full rounds
//! round *up* and the sum saturates.
//!
//! One step of 255 is inside the one-step bound ADR 0006 states for the whole device path,
//! and the default is 16 where the answer is exact — but it is a real cost of a short tail,
//! and it is written down here rather than left for whoever next changes the count.
//!
//! # What is deliberately not gated here
//!
//! A *partly* covered pixel's value at a short tail. Its exact answer is a function of
//! where the grid puts its samples and is supposed to differ between counts — `4/9` of a
//! pixel and `7/16` of it are both correct — so any bound wide enough to be true is wider
//! than the whole error a dropped tail causes, and a test written to it passes with the
//! tail dropped. That was tried: with `chunks_exact` in place of `chunks`, an edge pixel
//! moves by less than `255/side`, which is exactly the bound such a test has to allow. The
//! wholly covered pixel is the sharp probe, and it is the one below.

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

use quorra_gpu::{Coverage, Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, Paint, Point, Scene, SceneBuilder, Segment,
};

mod common;

use common::probe::alpha;

/// The fixture in its own units, magnified so that its tile is far too large for the atlas
/// below and the lane chooser sends it to the device (the recipe `coverage_lanes.rs` uses,
/// for the same reason).
const UNITS: u32 = 48;
const MAGNIFY: u32 = 16;
const SIZE: u32 = UNITS * MAGNIFY;

/// A small atlas, so the shape reaches the device lane rather than being cached in front
/// of it.
const TINY_ATLAS: u64 = 64 * 1024;

/// Every sample count a caller can reach. `Options::coverage_samples` is clamped to
/// `4..=64` and rounded down to a perfect square at construction, so this list is the whole
/// reachable set — and three of its seven entries (9, 25, 49) leave a last group of one.
const REQUESTED: [u32; 7] = [4, 9, 16, 25, 36, 49, 64];

fn device_with(samples: u32) -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        coverage: Coverage::Gpu,
        coverage_samples: samples,
        atlas_budget: TINY_ATLAS,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

/// A straight-edged quadrilateral that is not an axis-aligned rectangle, so the recogniser
/// refuses it and the fill goes down a coverage lane. Its interior at unit `(24, 24)` is
/// far from every edge at this magnification.
fn slab(device: &mut Device) -> Scene {
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(8.0, 8.0)),
            Segment::LineTo(Point::new(40.0, 10.0)),
            Segment::LineTo(Point::new(40.0, 40.0)),
            Segment::LineTo(Point::new(8.0, 38.0)),
            Segment::Close,
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0)),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    builder.finish()
}

fn render(samples: u32) -> Vec<u8> {
    let mut device = device_with(samples);
    device.wait_until_warm();
    let scene = slab(&mut device);
    device
        .render(
            &scene,
            &Viewport::full(SIZE, SIZE, Affine::scale(MAGNIFY as f32, MAGNIFY as f32)),
            Target::Readback,
        )
        .expect("the scene is inside every budget")
        .into_raster()
        .unwrap()
        .into_pixels()
}

/// The alpha at the middle of unit cell `(x, y)`.
///
/// `coverage_lanes.rs` carries the same probe over the same `UNITS × MAGNIFY` fixture; both
/// now read through `common::probe::alpha`, and what stays here is the *unit-to-device* map,
/// which is this fixture's own arithmetic rather than a raster's.
fn at_unit(pixels: &[u8], x: u32, y: u32) -> u8 {
    alpha(
        pixels,
        SIZE,
        x * MAGNIFY + MAGNIFY / 2,
        y * MAGNIFY + MAGNIFY / 2,
    )
}

/// **A sample count that is not a multiple of the pass width still covers a covered
/// pixel**, and still leaves an uncovered one alone.
///
/// Nine, twenty-five and forty-nine each leave a last group of one sample against a pass
/// that writes four channels. Both halves of the tail's answer — the clear that zeroes the
/// channels nobody wrote, and the divisor that is the sample count rather than four times
/// the group count — are asserted here at once, because a wholly covered pixel reaches
/// full coverage only if both hold.
///
/// The bound is derived rather than fitted: the sheet is R8, each of the `g = ceil(n/4)`
/// rounds deposits its share rounded to a byte, so the sum is at most `g/2` steps short of
/// full. What the lane actually produces is in the module comment; the gate is the bound
/// the arithmetic gives.
#[test]
fn a_sample_count_that_is_not_a_multiple_of_the_pass_width_still_covers() {
    for samples in REQUESTED {
        let pixels = render(samples);
        let rounds = samples.div_ceil(4);
        let floor = 255 - u8::try_from(rounds.div_ceil(2)).unwrap();
        let interior = at_unit(&pixels, 24, 24);
        assert!(
            interior >= floor,
            "a wholly covered pixel reads {interior} at {samples} samples, below the {floor} \
             that {rounds} rounds of byte-rounded shares can account for"
        );
        assert_eq!(
            at_unit(&pixels, 2, 2),
            0,
            "a wholly uncovered pixel at {samples} samples"
        );
    }
}

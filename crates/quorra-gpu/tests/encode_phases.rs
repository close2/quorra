//! The subdivision of `encode`, and the two host-side steps beside it.
//!
//! ADR 0023, from the caller's feedback §13: `encode` is 45% of their page turn, tracks
//! the scene's size at 3.86 µs a command, and was a single number — *"whether those
//! 3.86 µs a command are path flattening, bind-group churn, buffer writes, sorting, or
//! `wgpu`'s own command recording is invisible from here"*.
//!
//! What these tests hold is what an instrument has to be worth trusting:
//!
//! - **It is off unless asked for**, because the measurement costs a clock read at each
//!   seam and would otherwise be three times the encode it measures on a page of
//!   rectangles.
//! - **The parts add up to the whole**, so a caller can subtract with confidence.
//! - **It attributes**: a page whose work is path geometry must show it in `geometry`,
//!   and a page of rectangles — which flattens nothing and packs nothing — must not.
//! - **The acquire and the present are named**, so the remainder a host computes
//!   against its own clock is small enough to trust, and `host_total` says which
//!   phases that subtraction may use.

// Test-file lint policy as in m1.rs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, Paint, Point, Rect, Scene, SceneBuilder, Segment,
};

const W: u32 = 512;
const H: u32 = 512;

fn device(instrument_encode: bool) -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        instrument_encode,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

/// Rectangles: the analytic lane, which flattens nothing and packs nothing.
fn rectangles() -> Scene {
    let mut builder = SceneBuilder::new();
    for i in 0..400_u32 {
        let x = f32::from(u16::try_from(i % 20).unwrap()) * 25.0;
        let y = f32::from(u16::try_from(i / 20).unwrap()) * 25.0;
        builder
            .rect(
                Rect::new(Point::new(x, y), Point::new(x + 20.0, y + 20.0)),
                Affine::IDENTITY,
                Color::new(0.1, 0.2, 0.3, 1.0),
                None,
                None,
            )
            .unwrap();
    }
    builder.finish()
}

/// Curves large enough to miss the atlas, so every one of them flattens and packs.
fn curves(device: &mut Device) -> Scene {
    let mut builder = SceneBuilder::new();
    for i in 0..40_u32 {
        let cx = f32::from(u16::try_from(i % 8).unwrap()) * 60.0 + 40.0;
        let cy = f32::from(u16::try_from(i / 8).unwrap()) * 100.0 + 60.0;
        let r = 45.0;
        let outline = device
            .upload_outline(&[
                Segment::MoveTo(Point::new(cx - r, cy)),
                Segment::CubicTo {
                    c1: Point::new(cx - r, cy - r),
                    c2: Point::new(cx + r, cy - r),
                    to: Point::new(cx + r, cy),
                },
                Segment::CubicTo {
                    c1: Point::new(cx + r, cy + r),
                    c2: Point::new(cx - r, cy + r),
                    to: Point::new(cx - r, cy),
                },
                Segment::Close,
            ])
            .unwrap();
        builder
            .fill(
                outline,
                Affine::IDENTITY,
                FillRule::NonZero,
                Paint::Solid(Color::new(0.2, 0.3, 0.7, 1.0)),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
    }
    builder.finish()
}

fn phase(phases: &[(&'static str, Duration)], name: &str) -> Option<Duration> {
    phases
        .iter()
        .find(|(label, _)| *label == name)
        .map(|(_, duration)| *duration)
}

/// Off by default, and the frame says nothing about encode's parts.
#[test]
fn the_subdivision_is_absent_until_it_is_asked_for() {
    let mut device = device(false);
    let scene = curves(&mut device);
    let frame = device
        .render(
            &scene,
            &Viewport::full(W, H, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders");
    let phases = &frame.timings().phases;
    assert!(
        phase(phases, "encode: geometry").is_none(),
        "the default frame must not pay for a measurement nobody asked for: {phases:?}"
    );
}

/// Asked for, the three parts are there and they add up to `encode` exactly.
#[test]
fn the_parts_add_up_to_the_whole() {
    let mut device = device(true);
    let scene = curves(&mut device);
    let frame = device
        .render(
            &scene,
            &Viewport::full(W, H, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders");
    let timings = frame.timings();
    let geometry = phase(&timings.phases, "encode: geometry").expect("geometry is reported");
    let staging = phase(&timings.phases, "encode: staging").expect("staging is reported");
    let recording = phase(&timings.phases, "encode: recording").expect("recording is reported");

    assert_eq!(
        geometry.saturating_add(staging).saturating_add(recording),
        timings.encode,
        "the subdivision must be a partition of encode, not an overlapping sample"
    );
    assert!(
        geometry > Duration::ZERO,
        "forty flattened and rasterised curves must show as geometry"
    );
    assert!(
        staging > Duration::ZERO,
        "forty tiles packed onto the sheet must show as staging"
    );
}

/// And it attributes: the analytic lane flattens nothing and packs nothing, so a page
/// of rectangles must show its encode as recording rather than geometry.
#[test]
fn rectangles_are_recording_and_curves_are_geometry() {
    let mut device = device(true);

    let rects = rectangles();
    let frame = device
        .render(
            &rects,
            &Viewport::full(W, H, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders");
    let geometry = phase(&frame.timings().phases, "encode: geometry").unwrap();
    let staging = phase(&frame.timings().phases, "encode: staging").unwrap();
    assert_eq!(
        (geometry, staging),
        (Duration::ZERO, Duration::ZERO),
        "the rectangle lane touches neither the rasteriser nor the sheet"
    );

    let scene = curves(&mut device);
    let frame = device
        .render(
            &scene,
            &Viewport::full(W, H, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders");
    let curved = phase(&frame.timings().phases, "encode: geometry").unwrap();
    assert!(
        curved > Duration::ZERO,
        "the same instrument must find the work when there is work to find"
    );
}

/// The other half of §13: the two host-side steps outside the three phases are named,
/// and `host_total` says which numbers a caller may subtract from its own clock.
#[test]
fn the_acquire_and_the_present_are_named_and_the_host_phases_sum() {
    let mut device = device(false);
    let scene = rectangles();
    let frame = device
        .render(
            &scene,
            &Viewport::full(W, H, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders");
    let timings = frame.timings();
    assert!(
        phase(&timings.phases, "target acquire").is_some(),
        "the acquire is a host-side step and must not hide in a remainder: {:?}",
        timings.phases
    );
    assert!(
        phase(&timings.phases, "present").is_some(),
        "so must the present: {:?}",
        timings.phases
    );
    assert_eq!(
        timings.host_total(),
        timings
            .encode
            .saturating_add(timings.upload)
            .saturating_add(timings.readback),
        "host_total is exactly the three phases measured on the caller's clock — \
         execute is the adapter's and may not be mixed in"
    );
}

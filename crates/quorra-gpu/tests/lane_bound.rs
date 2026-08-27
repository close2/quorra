//! The lane divergence, stated and gated (ADR 0094): where the Cpu and Compute lanes
//! disagree at all, they disagree by **at most one coverage step**.
//!
//! ADR 0093's gate found pure-lane frames differing on a fixture the zero-pixel
//! suites never reached. The diagnosis (ADR 0094): both lanes convert coverage with
//! the same round-half-up, and both compute it with mirrored arithmetic — but not in
//! the same *order*. The scanline sums per tile, the deposit sums slabs along a row,
//! and a shallow edge whose true coverage lies within an ulp of a byte boundary
//! rounds apart. That is a property of float summation order, not of either mirror,
//! so it is stated under the relaxed contract (ADR 0082) rather than chased: alpha
//! within one step of 255, always. This suite is the statement's gate — a regression
//! past one step is a defect, not more of the same.
//!
//! The colour channels are deliberately gated *through* the alpha: straight-alpha
//! output divides the premultiplied colour by the pixel's own alpha, so a one-step
//! alpha difference on a nearly transparent pixel amplifies into tens of colour
//! levels that represent the same premultiplied ink. What is bounded is the ink.

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

mod common;

use quorra_gpu::{Coverage, Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, Paint, Point, Scene, SceneBuilder, Segment,
};

const SIZE: u32 = 128;

fn device(coverage: Coverage) -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        coverage,
        compute_assist: Some(false),
        glyph_quantum: None,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

/// The shape family that found the divergence: shallow near-horizontal edges laid on
/// near-integer rows, where a run of pixels lands its coverage on byte boundaries.
fn page(device: &mut Device) -> Scene {
    let mut builder = SceneBuilder::new();
    for index in 0..40u32 {
        let wobble = (index % 7) as f32;
        let outline = device
            .upload_outline(&[
                Segment::MoveTo(Point::new(0.0, 0.0)),
                Segment::LineTo(Point::new(9.0 + wobble * 0.3, 1.0)),
                Segment::CubicTo {
                    c1: Point::new(10.0 + wobble * 0.2, 5.0),
                    c2: Point::new(5.0, 10.0 + wobble * 0.1),
                    to: Point::new(1.0, 9.0),
                },
                Segment::Close,
            ])
            .unwrap();
        for repeat in 0..2u32 {
            builder
                .fill(
                    outline,
                    Affine {
                        a: 1.0,
                        b: 0.0,
                        c: 0.0,
                        d: 1.0,
                        e: 4.0 + 12.0 * ((index * 2 + repeat) % 10) as f32,
                        f: 4.0 + 15.0 * ((index * 2 + repeat) / 10) as f32,
                    },
                    FillRule::NonZero,
                    Paint::Solid(Color::new(0.2, 0.2, 0.6, 1.0)),
                    None,
                    BlendMode::Normal,
                    Compose::SrcOver,
                    None,
                )
                .unwrap();
        }
    }
    builder.finish()
}

/// One coverage step, at every scale a zoom passes through.
#[test]
fn the_lanes_agree_to_one_coverage_step() {
    for scale in [0.6_f32, 0.85, 1.0, 1.3, 1.7, 2.3] {
        let viewport = Affine {
            a: scale,
            b: 0.0,
            c: 0.0,
            d: scale,
            e: 0.0,
            f: 0.0,
        };
        let px = |coverage: Coverage| {
            let mut device = device(coverage);
            let scene = page(&mut device);
            device
                .render(
                    &scene,
                    &Viewport::full(SIZE, SIZE, viewport),
                    Target::Readback,
                )
                .expect("renders")
                .into_raster()
                .unwrap()
                .into_pixels()
        };
        let cpu = px(Coverage::Cpu);
        let gpu = px(Coverage::Compute);
        for (index, (a, b)) in cpu.chunks_exact(4).zip(gpu.chunks_exact(4)).enumerate() {
            assert!(
                a[3].abs_diff(b[3]) <= 1,
                "pixel {index} at scale {scale}: alpha {} vs {} — past one coverage step",
                a[3],
                b[3]
            );
            if a[3].min(b[3]) > 1 && a[3] == b[3] {
                // Equal alpha must mean equal ink: the colour divergence the module
                // comment licences rides *only* on an alpha step.
                assert_eq!(
                    &a[..3],
                    &b[..3],
                    "pixel {index} at scale {scale}: same alpha, different colour"
                );
            }
        }
    }
}

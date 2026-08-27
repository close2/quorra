//! The growth road, forced (ADR 0095): a first emit that meets its capacity grows to
//! the scanned total, re-runs whole, and draws the steady road's bytes — before
//! anything is presented.
//!
//! The steady road never overflows on ordinary pages, so this suite compiles the
//! seam in (`--features sabotage-capacity`), starts the persistent capacity at one
//! edge, and holds the re-run to byte identity with an unsabotaged device.

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
#![cfg(feature = "sabotage-capacity")]

mod common;

use quorra_gpu::{Coverage, Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, Paint, Point, Scene, SceneBuilder, Segment,
};

const SIZE: u32 = 96;

fn device() -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        coverage: Coverage::Compute,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

fn page(device: &mut Device) -> Scene {
    let curvy = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(10.0, 20.0)),
            Segment::CubicTo {
                c1: Point::new(40.0, -10.0),
                c2: Point::new(60.0, 90.0),
                to: Point::new(86.0, 30.0),
            },
            Segment::CubicTo {
                c1: Point::new(70.0, 70.0),
                c2: Point::new(30.0, 80.0),
                to: Point::new(10.0, 20.0),
            },
            Segment::Close,
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            curvy,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Solid(Color::new(0.2, 0.5, 0.8, 1.0)),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    builder.finish()
}

fn render(sabotage: bool) -> Vec<u8> {
    if sabotage {
        unsafe { std::env::set_var("QUORRA_SABOTAGE_CAPACITY", "1") };
    } else {
        unsafe { std::env::remove_var("QUORRA_SABOTAGE_CAPACITY") };
    }
    let mut device = device();
    let scene = page(&mut device);
    let pixels = device
        .render(
            &scene,
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the growth road still draws")
        .into_raster()
        .unwrap()
        .into_pixels();
    unsafe { std::env::remove_var("QUORRA_SABOTAGE_CAPACITY") };
    pixels
}

/// The claim itself: a capacity of one edge overflows, grows, re-runs, and the frame
/// is the steady road's bytes.
#[test]
fn an_overflowed_capacity_grows_and_draws_the_same_bytes() {
    let steady = render(false);
    let grown = render(true);
    assert!(
        steady.iter().any(|&b| b != 0),
        "the fixture draws something, or the comparison is vacuous"
    );
    assert_eq!(
        grown, steady,
        "the growth road is the same picture, only once"
    );
}

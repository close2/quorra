//! A frame draws what its scene says, whatever frames came before it on that device.
//!
//! Everything a device keeps between frames — the glyph atlas, the winding texture, the
//! pipeline store — is a cache, and a cache is only sound while it is invisible. So the
//! shape of every test here is the same: render a scene on a device that has already
//! drawn something *else*, and require the pixels to equal what a device that had drawn
//! nothing produces. A difference is not a tolerance, it is a defect: the same scene
//! reached both.
//!
//! The order that matters is **large then small**, because that is the direction a cache
//! grows in and the one a viewer walks when a person zooms in and back out. It is also
//! the direction the caller's `QUORRA_FEEDBACK.md` §11 reported from the window: a page
//! magnified past 1000% and brought back drew one letter as another, and kept doing it.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Coverage, Device, Options, Target, Viewport};
use quorra_scene::{Affine, BlendMode, Compose, FillRule, Point, Scene, SceneBuilder, Segment};

mod common;

use common::scene::black;

/// The side of the target the compared frame is drawn at.
const SMALL: u32 = 48;
/// The side of the target the frame *before* it is drawn at — large enough that its
/// coverage tiles pack a taller scratch sheet than the small frame's do.
const LARGE: u32 = 240;

fn device(coverage: Coverage) -> Device {
    let device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        coverage,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs");
    device.wait_until_warm();
    device
}

/// A curve scaled to `side`, past `MAX_GLYPH_DIM` at either size, so no atlas stands in
/// front of it and its coverage lands on the frame's scratch sheet.
fn blob(device: &mut Device, side: u32) -> Scene {
    let s = side as f32 / 240.0;
    let at = |x: f32, y: f32| Point::new(x * s, y * s);
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(at(20.0, 20.0)),
            Segment::CubicTo {
                c1: at(220.0, 4.0),
                c2: at(236.0, 160.0),
                to: at(120.0, 220.0),
            },
            Segment::CubicTo {
                c1: at(40.0, 236.0),
                c2: at(4.0, 120.0),
                to: at(20.0, 20.0),
            },
            Segment::Close,
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            black(),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    builder.finish()
}

fn draw(device: &mut Device, side: u32) -> Vec<u8> {
    let scene = blob(device, side);
    device
        .render(
            &scene,
            &Viewport::full(side, side, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the scene is inside every budget")
        .into_raster()
        .unwrap()
        .into_pixels()
}

/// The GPU coverage lane keeps one winding texture across frames and grows it to the
/// largest sheet any frame has needed (`winding.rs` states the measurement that bought
/// the reuse: allocating and zeroing it per frame cost 10.7 ms of a 15 ms frame). A
/// frame whose sheet is *shorter* than that texture must still draw its own coverage.
///
/// This is §11 of the caller's feedback, made small: their ladder went wrong at 6400%
/// and stayed wrong at 1600% on the way down, while 3200% — whose sheet was the tallest
/// the device had seen — stayed right. Two frames on one device is the whole recipe.
#[test]
fn a_short_gpu_frame_after_a_tall_one_draws_its_own_coverage() {
    let alone = draw(&mut device(Coverage::Gpu), SMALL);

    let mut device = device(Coverage::Gpu);
    let large = draw(&mut device, LARGE);
    let after = draw(&mut device, SMALL);

    assert!(
        large.iter().skip(3).step_by(4).any(|&a| a > 0),
        "the frame that goes first draws something, or it warms nothing"
    );
    assert_eq!(
        after, alone,
        "the same scene, drawn on a device that had drawn a larger frame"
    );
}

/// The same ordering under the CPU lane, which shares the scratch sheet but makes no
/// winding texture — the control that says the defect above is the lane's and not the
/// sheet's.
#[test]
fn a_short_cpu_frame_after_a_tall_one_draws_its_own_coverage() {
    let alone = draw(&mut device(Coverage::Cpu), SMALL);

    let mut device = device(Coverage::Cpu);
    let _ = draw(&mut device, LARGE);
    let after = draw(&mut device, SMALL);

    assert_eq!(after, alone);
}

//! Two rasters of one page, from one device — §11.4.7's four-component blending space.
//!
//! The caller's feedback §17 asks for one of two things, and says the section closes
//! with no change if the second is already true:
//!
//! > - a way to render the same viewport twice and read both back within one frame, or
//! > - two calls whose device state, caches and resources are not thrown away between
//! >   them.
//!
//! The second is true, and this file is the evidence rather than the assertion. §11.3.4
//! applies the compositing formula per component, so four components are three plus one:
//! the page is interpreted twice — once carrying the additive complements of cyan,
//! magenta and yellow, once the complement of black — and the two rasters are put back
//! together afterwards by a per-pixel conversion that has nothing to do with
//! rasterisation.
//!
//! What that needs from a device is that the second pass finds the first pass's work
//! still there. Resources are device-scoped and uploaded once; the glyph atlas is keyed
//! by `(outline, linear part, phase, rule)` and **not** by colour, so the second
//! interpretation of the same geometry hits every tile the first one rasterised.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, OutlineId, Paint, Point, Scene, SceneBuilder,
    Segment,
};

const W: u32 = 256;
const H: u32 = 256;

fn device() -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        instrument_encode: true,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

/// Letterform-shaped outlines, uploaded once and referenced by both interpretations.
fn outlines(device: &mut Device, count: u32) -> Vec<OutlineId> {
    (0..count)
        .map(|k| {
            let r = 6.0 + (k % 5) as f32;
            device
                .upload_outline(&[
                    Segment::MoveTo(Point::new(-r, 0.0)),
                    Segment::CubicTo {
                        c1: Point::new(-r, -r),
                        c2: Point::new(r, -r),
                        to: Point::new(r, 0.0),
                    },
                    Segment::CubicTo {
                        c1: Point::new(r, r * 1.5),
                        c2: Point::new(-r, r * 1.5),
                        to: Point::new(-r, 0.0),
                    },
                    Segment::Close,
                ])
                .unwrap()
        })
        .collect()
}

/// One interpretation of the page: the same geometry, a different set of components.
fn interpretation(outlines: &[OutlineId], ink: Color) -> Scene {
    let mut builder = SceneBuilder::new();
    for i in 0..240_u32 {
        let x = (i % 16) as f32 * 15.0 + 10.0;
        let y = (i / 16) as f32 * 16.0 + 12.0;
        builder
            .fill(
                outlines[(i as usize) % outlines.len()],
                Affine::translate(x, y),
                FillRule::NonZero,
                Paint::Solid(ink),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
    }
    builder.finish()
}

fn phase(frame: &quorra_gpu::Frame, name: &str) -> std::time::Duration {
    frame
        .timings()
        .phases
        .iter()
        .find(|(label, _)| *label == name)
        .map_or(std::time::Duration::ZERO, |(_, d)| *d)
}

/// Both rasters come back, they are different pictures, and the second pass pays no
/// geometry at all.
#[test]
fn a_second_interpretation_costs_no_geometry() {
    let mut device = device();
    let outlines = outlines(&mut device, 40);
    let cmy = interpretation(&outlines, Color::new(0.2, 0.6, 0.9, 1.0));
    let black = interpretation(&outlines, Color::new(0.05, 0.05, 0.05, 1.0));
    let viewport = Viewport::full(W, H, Affine::IDENTITY);

    let first = device
        .render(&cmy, &viewport, Target::Readback)
        .expect("pass one");
    let first_geometry = phase(&first, "encode: geometry");
    let first_pixels = first.into_raster().unwrap().into_pixels();

    let second = device
        .render(&black, &viewport, Target::Readback)
        .expect("pass two, on the same device");
    let second_geometry = phase(&second, "encode: geometry");
    let second_pixels = second.into_raster().unwrap().into_pixels();

    assert_eq!(first_pixels.len(), (W * H * 4) as usize);
    assert_eq!(second_pixels.len(), (W * H * 4) as usize);
    assert_ne!(
        first_pixels, second_pixels,
        "the two interpretations carry different components and must differ"
    );

    assert!(
        first_geometry > std::time::Duration::ZERO,
        "the first pass rasterises the page's outlines"
    );
    assert_eq!(
        second_geometry,
        std::time::Duration::ZERO,
        "and the second finds every one of them in the atlas: the key is \
         (outline, linear part, phase, rule) and colour is not in it, which is what \
         makes §11.4.7's second interpretation nearly free"
    );
}

/// Resources are device-scoped: one upload, referenced by both display lists, and the
/// second pass schedules no transfer for them.
#[test]
fn the_two_passes_share_one_upload() {
    let mut device = device();
    let outlines = outlines(&mut device, 40);
    let viewport = Viewport::full(W, H, Affine::IDENTITY);

    let first = device
        .render(
            &interpretation(&outlines, Color::new(0.2, 0.6, 0.9, 1.0)),
            &viewport,
            Target::Readback,
        )
        .expect("pass one");
    let first_bytes = first.counters().bytes_uploaded;
    drop(first);

    let second = device
        .render(
            &interpretation(&outlines, Color::new(0.05, 0.05, 0.05, 1.0)),
            &viewport,
            Target::Readback,
        )
        .expect("pass two");
    assert!(
        second.counters().bytes_uploaded < first_bytes,
        "the second pass must not re-upload what the first made resident: \
         {} against {first_bytes}",
        second.counters().bytes_uploaded
    );
}

/// The passes do not contaminate each other: each raster equals the same scene rendered
/// on a device that has drawn nothing else.
#[test]
fn neither_pass_changes_what_the_other_draws() {
    let viewport = Viewport::full(W, H, Affine::IDENTITY);
    let inks = [
        Color::new(0.2, 0.6, 0.9, 1.0),
        Color::new(0.05, 0.05, 0.05, 1.0),
    ];

    let mut together = device();
    let shared = outlines(&mut together, 40);
    let paired: Vec<Vec<u8>> = inks
        .iter()
        .map(|ink| {
            together
                .render(&interpretation(&shared, *ink), &viewport, Target::Readback)
                .expect("renders")
                .into_raster()
                .unwrap()
                .into_pixels()
        })
        .collect();

    for (index, ink) in inks.iter().enumerate() {
        let mut alone = device();
        let own = outlines(&mut alone, 40);
        let solo = alone
            .render(&interpretation(&own, *ink), &viewport, Target::Readback)
            .expect("renders")
            .into_raster()
            .unwrap()
            .into_pixels();
        assert_eq!(
            paired[index], solo,
            "interpretation {index} drawn beside the other must equal itself drawn alone"
        );
    }
}

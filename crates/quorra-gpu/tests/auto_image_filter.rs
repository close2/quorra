//! `ImageFilter::Auto` (ADR 0089): the filter and the reduction resolved per
//! placement, so one scene with a picture on it is true at every viewport.
//!
//! The claim is the caller's ADR 0702 dependency: a page-space scene survives zooming
//! only if nothing in it read the view, and an image command used to carry a
//! filter-and-reduction answer that was read off exactly one placement. Under `Auto`
//! the same scene minified draws the area-averaged variant and magnified draws flat
//! rectangles — decided at encode, held here at the pixel.

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

use std::sync::Arc;

use common::headless::{device, pixels};
use quorra_gpu::{Device, Target, Viewport};
use quorra_scene::{Affine, BlendMode, ImageFilter, ImageSpec, Scene, SceneBuilder};

const SIZE: u32 = 64;

/// An 8×8 checkerboard: pure black and white, opaque. Any pixel of the reduced
/// variant is their mean; any pixel of the unreduced one is one or the other.
fn checkerboard(device: &mut Device) -> quorra_scene::ImageId {
    let mut data = Vec::new();
    for y in 0..8u32 {
        for x in 0..8u32 {
            let v = if (x + y) % 2 == 0 { 255 } else { 0 };
            data.extend_from_slice(&[v, v, v, 255]);
        }
    }
    device
        .upload_image(&ImageSpec {
            width: 8,
            height: 8,
            data: Arc::from(data),
        })
        .unwrap()
}

/// One scene: the image over the unit square scaled to `extent` device pixels.
fn scene(device: &mut Device, extent: f32) -> Scene {
    let image = checkerboard(device);
    let mut builder = SceneBuilder::new();
    builder
        .image(
            image,
            Affine {
                a: extent,
                b: 0.0,
                c: 0.0,
                d: extent,
                e: 8.0,
                f: 8.0,
            },
            1.0,
            ImageFilter::Auto { interpolate: false },
            None,
            BlendMode::Normal,
            None,
        )
        .unwrap();
    builder.finish()
}

fn alpha_at(bytes: &[u8], x: u32, y: u32, channel: usize) -> u8 {
    bytes[((y * SIZE + x) * 4) as usize + channel]
}

/// Minified fourfold, every drawn pixel is the blocks' mean — the area-averaged
/// variant, resolved and realised by the encode, nothing pre-reduced in the scene.
#[test]
fn a_minified_auto_image_draws_the_area_averaged_variant() {
    let mut device = device();
    let built = scene(&mut device, 2.0);
    let bytes = pixels(
        device
            .render(
                &built,
                &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
                Target::Readback,
            )
            .expect("renders"),
    );
    // The image covers device pixels 8..10 in both axes; each drawn pixel gathers a
    // 4×4 block whose mean is exactly 128 (two black, two white per row, rounded up).
    for (x, y) in [(8, 8), (9, 8), (8, 9), (9, 9)] {
        assert_eq!(
            alpha_at(&bytes, x, y, 0),
            128,
            "({x},{y}): the mean of the block, not one sample of it"
        );
    }
}

/// The same construction magnified: `/Interpolate` false draws flat rectangles, so a
/// pixel well inside a black cell is black — no reduction, no filtering.
#[test]
fn a_magnified_auto_image_draws_flat_rectangles() {
    let mut device = device();
    let built = scene(&mut device, 48.0);
    let bytes = pixels(
        device
            .render(
                &built,
                &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
                Target::Readback,
            )
            .expect("renders"),
    );
    // The unit square carries the image with its top row at unit y = 1 (§8.9.5),
    // so the top-left *drawn* cell is image row 7: (0 + 7) % 2 = 1, black. Its
    // centre is far from any neighbour.
    assert_eq!(alpha_at(&bytes, 11, 11, 0), 0);
    // One cell to the right: white.
    assert_eq!(alpha_at(&bytes, 17, 11, 0), 255);
}

/// The whole point: **one scene**, rendered at a magnifying and a minifying viewport,
/// answers each placement with that placement's own resolution — no rebuild between.
#[test]
fn one_scene_resolves_per_viewport() {
    let mut device = device();
    let built = scene(&mut device, 8.0);
    // Magnified 4×: the image covers 32..64-ish; a point inside the first white cell.
    let magnified = pixels(
        device
            .render(
                &built,
                &Viewport::full(
                    SIZE,
                    SIZE,
                    Affine {
                        a: 4.0,
                        b: 0.0,
                        c: 0.0,
                        d: 4.0,
                        e: -24.0,
                        f: -24.0,
                    },
                ),
                Target::Readback,
            )
            .expect("renders"),
    );
    // Image row 7 at the top of the drawn square (§8.9.5's flip): black first.
    assert_eq!(alpha_at(&magnified, 10, 10, 0), 0, "flat black cell");
    assert_eq!(alpha_at(&magnified, 14, 10, 0), 255, "flat white cell");
    // Minified 4× (device extent 2): the mean.
    let minified = pixels(
        device
            .render(
                &built,
                &Viewport::full(
                    SIZE,
                    SIZE,
                    Affine {
                        a: 0.25,
                        b: 0.0,
                        c: 0.0,
                        d: 0.25,
                        e: 6.0,
                        f: 6.0,
                    },
                ),
                Target::Readback,
            )
            .expect("renders"),
    );
    assert_eq!(alpha_at(&minified, 8, 8, 0), 128, "the blocks' mean");
}

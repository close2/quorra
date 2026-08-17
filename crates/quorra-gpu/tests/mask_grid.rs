//! A mask whose grid is not the image's: ISO 32000-2 §8.9.6.3, §8.9.5.1, §11.5.
//!
//! The question comes from the caller's reading of hayro #1315, #1319 and #2 — a 450 × 588
//! JPEG whose `/Mask` is a 1350 × 1763 JBIG2 image mask, which is the ordinary case rather
//! than the exotic one. Two costs are measured in those issues, and the caller's document
//! is explicit that only the second is quorra's: decoding a packed mask, and **drawing
//! through one whose grid does not match**.
//!
//! # The clause, and a correction to its number
//!
//! The permission is **§8.9.6.3 Explicit masking**, not §8.9.6.4 — §8.9.6.4 is colour key
//! masking, which is a range of colours and says nothing about resolution. The sentence is:
//!
//! > The base image and the image mask need not have the same resolution ( Width and
//! > Height values), but since all images shall be defined on the unit square in user
//! > space, their boundaries on the page will coincide; that is, they will overlay each
//! > other.
//!
//! and the "defined on the unit square" it leans on is §8.9.5.1's:
//!
//! > The correspondence between image space and user space is constant: the unit square of
//! > user space, bounded by user coordinates (0, 0) and (1, 1), corresponds to the boundary
//! > of the image in image space
//!
//! # What reaches us, and therefore what these gates are
//!
//! **A `/Mask` cannot reach this library as a second raster**: `ImageSpec` is one
//! straight-alpha RGBA8 buffer on one grid, `Command::Image` names one `ImageId`, and the
//! only other attenuation an image command carries is a `MaskId` — which names a list of
//! *drawing commands* (§11.5's transparency group), not a raster with a grid of its own. So
//! a mismatch between two image grids is not expressible here, and the caller resolves it
//! upstream: `pdf-model`'s `combine_on_the_finer_grid` folds an explicit `/Mask` into the
//! base image's alpha on `max(image, mask)` in each axis at interpretation time, and a
//! `/SMask` whose refinement is unbuildable is deferred to `Grid::for_placement`'s device
//! resolution instead. What arrives is one already-composited raster and a `Nearest` or
//! `Linear` flag (integration note 1).
//!
//! That makes two gates, at two different depths:
//!
//! - **The boundary.** [`an_uploaded_image_is_one_raster_on_one_grid`] and
//!   [`an_image_command_carries_no_second_raster`] destructure the two types
//!   exhaustively, so a field added to either stops this file compiling. That is the gate
//!   the assumption above actually needs: the assumption is not "we sample two grids
//!   correctly", it is "there is only one grid", and only the type can hold that.
//! - **The mismatch we do have.** A soft mask *is* a second grid — §11.5 renders it at
//!   device resolution — so an image at a coarse grid under a soft mask is exactly "drawing
//!   through a mask whose grid does not match", in the one form that reaches us.
//!   [`a_soft_mask_edge_lands_on_the_device_grid_not_the_images`] holds it, and
//!   [`an_images_own_texel_boundary_lands_where_the_unit_square_puts_it`] holds the other
//!   side of the same mapping.

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

use common::headless::{device, render};
use quorra_scene::{
    Affine, BlendMode, Color, Command, ImageFilter, ImageSpec, MaskKind, Point, Rect, SceneBuilder,
};
use std::sync::Arc;

/// 64 pixels wide: 64 × 4 bytes = 256, the buffer-copy row alignment.
const SIZE: u32 = 64;

/// Where every image below is placed, in device pixels. 48 wide, so a 2 × 2 image's texel
/// boundary falls on 32 and a 4 × 4 image's on 20, 32 and 44 — none of which is where
/// [`MASK_EDGE`] is.
const PLACED: Rect = Rect {
    min: Point::new(8.0, 8.0),
    max: Point::new(56.0, 56.0),
};

/// The device x the soft mask's own rectangle ends at. Chosen to be **inside** a texel of
/// both a 2 × 2 and a 4 × 4 image placed at [`PLACED`]: 30 is 22 device pixels into the
/// first half and 10 into the second quarter, so a mask quantised to either image's grid
/// would put its edge at 20 or 32 and this test would see it.
const MASK_EDGE: f32 = 30.0;

/// The device row every probe reads. Well inside [`PLACED`] vertically.
const ROW: u32 = 24;

fn placement() -> Affine {
    Affine {
        a: PLACED.max.x - PLACED.min.x,
        b: 0.0,
        c: 0.0,
        d: PLACED.max.y - PLACED.min.y,
        e: PLACED.min.x,
        f: PLACED.min.y,
    }
}

/// A `w` × `h` image from one RGBA quadruple per sample, given row-major, top row first
/// (which is `ImageSpec`'s stated layout).
fn image_of(w: u32, h: u32, samples: &[[u8; 4]]) -> ImageSpec {
    assert_eq!(samples.len() as u32, w * h, "one sample per texel");
    ImageSpec {
        width: w,
        height: h,
        data: Arc::from(
            samples
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<u8>>()
                .into_boxed_slice(),
        ),
    }
}

fn alpha_at(pixels: &[u8], x: u32, y: u32) -> u8 {
    pixels[((y * SIZE + x) * 4 + 3) as usize]
}

fn rgb_at(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2]]
}

/// **The boundary gate.** An uploaded image is one raster on one grid — three fields, and
/// none of them is a second raster or a second pair of dimensions.
///
/// The destructuring is the gate rather than the assertions: `ImageSpec` is not
/// `#[non_exhaustive]`, so a fourth field added to it stops this file compiling, and
/// whoever adds it reads this comment. That is what stands behind the claim in this
/// module's header — §8.9.6.3's differing resolutions cannot be expressed here, so the
/// caller must go on resolving them, and neither side may assume the other did.
#[test]
fn an_uploaded_image_is_one_raster_on_one_grid() {
    let spec = image_of(2, 1, &[[255, 0, 0, 255], [0, 0, 255, 128]]);
    let ImageSpec {
        width,
        height,
        data,
    } = spec.clone();
    assert_eq!((width, height), (2, 1));
    // One grid's worth of samples, exactly: §8.9.6.3's second raster would need its own
    // `Width` and `Height`, and there is nowhere here to put them.
    assert_eq!(data.len() as u32, width * height * 4);
    assert!(spec.is_consistent());
    assert!(
        !ImageSpec {
            width: 2,
            height: 1,
            data: Arc::from(vec![0_u8; 4].into_boxed_slice()),
        }
        .is_consistent(),
        "a buffer that is not exactly one grid is refused rather than reinterpreted"
    );
}

/// The same gate one level up: an image *command* names one image and no second raster.
///
/// Its `mask` is a [`MaskId`](quorra_scene::MaskId) — §11.5's transparency group, defined
/// by drawing commands and realised at device resolution — so it has no grid of its own to
/// disagree with the image's. A future field carrying a stencil would stop this compiling.
#[test]
fn an_image_command_carries_no_second_raster() {
    let mut builder = SceneBuilder::new();
    builder
        .image(
            quorra_scene::ImageId(0),
            placement(),
            1.0,
            ImageFilter::Nearest,
            None,
            BlendMode::Normal,
            None,
        )
        .expect("valid image command");
    let scene = builder.finish();
    let [command] = scene.commands() else {
        panic!("one command");
    };
    let Command::Image {
        image,
        transform,
        alpha,
        filter,
        clip,
        blend,
        mask,
    } = command
    else {
        panic!("an image command");
    };
    assert_eq!(image.0, 0);
    assert_eq!(*transform, placement());
    // Bit-for-bit: the builder stores the alpha it was given, and a tolerance here would
    // hide a value that had been repaired on the way through.
    assert_eq!(alpha.to_bits(), 1.0_f32.to_bits());
    assert_eq!(*filter, ImageFilter::Nearest);
    assert!(clip.is_none() && mask.is_none());
    assert_eq!(*blend, BlendMode::Normal);
}

/// **The mismatch that does reach us.** An image on a 4 × 4 grid — each texel 12 device
/// pixels wide at [`PLACED`] — under a soft mask whose rectangle ends at [`MASK_EDGE`],
/// which is inside a texel rather than on a texel boundary.
///
/// §11.5.1 makes a soft mask a thing that "defines values that may vary across different
/// points on the page", and §11.5 realises it by rendering its group at device resolution.
/// So the mask's grid is the device's and the image's is its own, and the edge must land at
/// device x = 30 — not at 20 or 32, which is where it would land if the mask were sampled
/// on the image's grid, and not at 28 or 32 if it were sampled on any other coarse one.
#[test]
fn a_soft_mask_edge_lands_on_the_device_grid_not_the_images() {
    let mut device = device();
    let uniform = device
        .upload_image(&image_of(4, 4, &[[255, 255, 255, 255]; 16]))
        .expect("upload");
    let mut builder = SceneBuilder::new();
    let mask = builder
        .mask(MaskKind::Alpha, None, |mask| {
            mask.rect(
                Rect::new(PLACED.min, Point::new(MASK_EDGE, PLACED.max.y)),
                Affine::IDENTITY,
                Color::new(1.0, 1.0, 1.0, 1.0),
                None,
                None,
            )
        })
        .expect("valid mask");
    builder
        .image(
            uniform,
            placement(),
            1.0,
            ImageFilter::Nearest,
            None,
            BlendMode::Normal,
            Some(mask),
        )
        .expect("valid image command");
    let scene = builder.finish();
    let pixels = render(&mut device, &scene, SIZE, SIZE);

    let edge = MASK_EDGE as u32;
    assert_eq!(
        alpha_at(&pixels, edge - 1, ROW),
        255,
        "the last device pixel the mask admits is painted"
    );
    assert_eq!(
        alpha_at(&pixels, edge, ROW),
        0,
        "the first device pixel outside the mask is not"
    );
    // The two texel boundaries the edge is *not* on, in both directions: if the mask had
    // been sampled on the image's 4 × 4 grid, the transition would be at one of these.
    assert_eq!(alpha_at(&pixels, 21, ROW), 255, "not quantised down to 20");
    assert_eq!(alpha_at(&pixels, 33, ROW), 0, "not quantised up to 32");
}

/// The other side of the same mapping: an image's own texel boundary lands where
/// §8.9.5.1's unit square puts it, which for a 2 × 2 image at [`PLACED`] is device 32.
///
/// This is the half of hayro #1315 and #2 that is quorra's — drawing an image whose grid is
/// not the device's — measured on the grid rather than on the clock, because a placement
/// that is off by half a texel is a wrong picture and a slow one is only slow.
#[test]
fn an_images_own_texel_boundary_lands_where_the_unit_square_puts_it() {
    let mut device = device();
    let quadrants = device
        .upload_image(&image_of(
            2,
            2,
            &[
                [255, 0, 0, 255],
                [0, 255, 0, 255],
                [0, 0, 255, 255],
                [255, 255, 0, 255],
            ],
        ))
        .expect("upload");
    let mut builder = SceneBuilder::new();
    builder
        .image(
            quadrants,
            placement(),
            1.0,
            ImageFilter::Nearest,
            None,
            BlendMode::Normal,
            None,
        )
        .expect("valid image command");
    let scene = builder.finish();
    let pixels = render(&mut device, &scene, SIZE, SIZE);

    // The boundary is at device 32 in both axes: 8 + 48/2.
    let quadrant = |x: u32, y: u32| rgb_at(&pixels, x, y);
    let corners = [
        quadrant(31, 31),
        quadrant(33, 31),
        quadrant(31, 33),
        quadrant(33, 33),
    ];
    for (i, a) in corners.iter().enumerate() {
        for b in &corners[i + 1..] {
            assert_ne!(a, b, "the four texels are four colours: {corners:?}");
        }
    }
    // Uniform up to the boundary and from it: a nearest-sampled texel is constant across
    // the device pixels it covers, which is what "the grids need not match" costs.
    assert_eq!(
        quadrant(9, 9),
        quadrant(31, 31),
        "the first texel is one colour"
    );
    assert_eq!(quadrant(33, 33), quadrant(55, 55), "and so is the last");
}

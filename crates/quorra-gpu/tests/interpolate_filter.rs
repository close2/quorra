//! The filter decision arrives on the command, and this lane executes it rather than
//! re-taking it (ISO 32000-2 §8.9.5.3, `doc/PLAN.md` integration note 1).
//!
//! # Where the question comes from
//!
//! hayro #1310, by way of the caller's `doc/HAYRO_ISSUES_FOR_QUORRA.md` §4: a user found
//! that small chart marks came out worse than Cairo's, and their workaround was to force
//! image filtering on regardless of what the PDF says. §8.9.5.1's Table 87 entry makes
//! `/Interpolate` the document's request —
//!
//! > (Optional) A flag indicating whether image interpolation should be performed by a
//! > PDF processor (see 8.9.5.3, "Image interpolation"). Default value: false.
//!
//! — and §8.9.5.3 says outright what it is worth:
//!
//! > Image interpolation is an attempt to produce a smooth transition between adjacent
//! > sample values when rendering an image whose resolution is significantly lower than
//! > that of the output device. Setting the value of the Interpolate entry in an image
//! > dictionary to true, is a way for a PDF to declare to a PDF processor that a specific
//! > image might render better if interpolation is used for this particular image.
//! > However, this is only a hint, and a PDF processor may ignore it.
//!
//! So the clause leaves the *choice* to the processor — and the processor is the caller,
//! not us. Integration note 1 is the shape that follows: `Image::is_smoothed(placement)`
//! is a method of the **placement** upstream, so what reaches this library is a resolved
//! [`ImageFilter`] on the image *command*. Our whole obligation is to execute the
//! decision the command carries and never to substitute one of our own — neither by
//! forcing a filter on (the #1310 workaround) nor by smoothing a minified image behind
//! the document's back.
//!
//! # What each test can catch
//!
//! An "we honour it" claim asserted only where filtering is asked for is half a gate, so
//! every test here is a pair: the decision that says *smooth* must visibly smooth, and
//! the decision that says *do not* must introduce no value that is not a sample. The
//! minified fixture is the one that matches #1310 exactly — an image drawn smaller than
//! its own sample grid, which is where a renderer is most tempted to average on its own.
//!
//! **The caller's closing caution belongs here too**: *a quality complaint about small
//! marks is very often a scan-conversion defect wearing a filtering costume.* Nothing in
//! this file is evidence about scan conversion; `tests/scale_invariance.rs` and the
//! caller's hairline ask are where that question lives.
//!
//! `m7.rs` holds the two filters' arithmetic on its own (exact blocks under magnification,
//! a monotonic ramp under linear). This file holds the thing neither of those can see: the
//! two decisions applied to *one* placement, and both of them applied inside one scene.

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

use std::sync::Arc;

use quorra_scene::{Affine, BlendMode, ImageFilter, ImageId, ImageSpec, Scene, SceneBuilder};

mod common;

use common::headless::{device, render};

/// The image's sample grid: eight by eight, wider than any target below.
const SAMPLES: u32 = 8;

/// The target every fixture draws into. Five pixels for eight samples, so the placement
/// is a **minification** — #1310's case, where the reporter's small marks were.
const SIDE: u32 = 5;

/// A checkerboard of pure black and pure white samples.
///
/// Chosen because its every 2×2 neighbourhood averages to 127.5: any averaging at all —
/// a linear tap, an area average, a mip level — produces a value that is *not* a sample,
/// so "every pixel is one of the two samples" is a decidable claim about whether we
/// filtered. The two values are the extremes of the channel, so a wrong answer cannot be
/// hidden by a tolerance.
fn checkerboard() -> ImageSpec {
    let mut data = Vec::with_capacity((SAMPLES * SAMPLES * 4) as usize);
    for row in 0..SAMPLES {
        for col in 0..SAMPLES {
            let level = if (row + col) % 2 == 0 { 255 } else { 0 };
            data.extend_from_slice(&[level, level, level, 255]);
        }
    }
    ImageSpec {
        width: SAMPLES,
        height: SAMPLES,
        data: Arc::from(data.as_slice()),
    }
}

/// One image command filling the whole target, under the filter the test names.
fn placed(image: ImageId, filter: ImageFilter) -> Scene {
    let mut builder = SceneBuilder::new();
    builder
        .image(
            image,
            Affine::scale(SIDE as f32, SIDE as f32),
            1.0,
            filter,
            None,
            BlendMode::Normal,
            None,
        )
        .expect("a finite placement at alpha 1");
    builder.finish()
}

fn red_channel(pixels: &[u8]) -> Vec<u8> {
    pixels.iter().step_by(4).copied().collect()
}

/// The two decisions do not draw the same picture.
///
/// This is the anti-substitution gate in its bluntest form: if this library ever forced
/// filtering on — #1310's workaround, applied one layer too low — or ever quietly declined
/// to filter, the two rasters would be equal and this test is the only one in the tree
/// that would say so.
#[test]
fn the_two_decisions_at_one_placement_draw_different_pixels() {
    let mut device = device();
    let image = device.upload_image(&checkerboard()).expect("consistent");
    let nearest = render(
        &mut device,
        &placed(image, ImageFilter::Nearest),
        SIDE,
        SIDE,
    );
    let linear = render(&mut device, &placed(image, ImageFilter::Linear), SIDE, SIDE);
    assert_ne!(
        nearest, linear,
        "one placement under the two resolved filters must not draw one picture — a \
         renderer that forces filtering on, or that ignores a request for it, passes \
         every other image test in this tree and fails only here"
    );
}

/// `/Interpolate` false (§8.9.5.3's default, and Table 87's): every pixel carries a value
/// that is *a sample*, at a placement small enough that averaging would be tempting.
///
/// The expectation is derived from the mapping alone. §8.9.5's unit square puts the image
/// over the target exactly, `image.wgsl` fetches the texel containing the pixel centre,
/// and the fixture's samples are 0 and 255 — so an unaveraged answer is one of those two
/// and an averaged one is neither. Nothing here is a comparison against another renderer
/// (CLAUDE.md principle 5).
#[test]
fn a_minified_nearest_placement_invents_no_value_between_its_samples() {
    let mut device = device();
    let image = device.upload_image(&checkerboard()).expect("consistent");
    let pixels = render(
        &mut device,
        &placed(image, ImageFilter::Nearest),
        SIDE,
        SIDE,
    );
    let reds = red_channel(&pixels);
    assert_eq!(reds.len(), (SIDE * SIDE) as usize);
    for (at, &level) in reds.iter().enumerate() {
        assert!(
            level == 0 || level == 255,
            "pixel {at} reads {level}, which is not one of this image's samples: the \
             placement asked for no interpolation and something averaged anyway \
             (row of reds: {reds:?})"
        );
    }
    // A raster of one value would satisfy the loop above without the mapping doing
    // anything, so the fixture has to be shown to carry both samples.
    assert!(
        reds.contains(&0) && reds.contains(&255),
        "the fixture must sample both levels or the assertion above is vacuous: {reds:?}"
    );
}

/// `/Interpolate` true at the same placement: the smoothing the document asked for is
/// actually performed.
///
/// The control for the test above, and the half of the pair that fails if we ever answer
/// "nearest" to every command. Its claim is deliberately weak in the bytes — the sampler's
/// interpolation precision is the driver's, which ADR 0011 states — and strong in the
/// property: at least one pixel is strictly between the two sample values, which no
/// point-sampled raster of a two-valued image can produce.
#[test]
fn a_minified_linear_placement_smooths_as_the_document_asked() {
    let mut device = device();
    let image = device.upload_image(&checkerboard()).expect("consistent");
    let pixels = render(&mut device, &placed(image, ImageFilter::Linear), SIDE, SIDE);
    let reds = red_channel(&pixels);
    assert!(
        reds.iter().any(|&level| level > 0 && level < 255),
        "no pixel lies between the samples, so nothing interpolated: {reds:?}"
    );
}

/// **The decision is per command, not per image and not per device** — integration
/// note 1's whole point, and the property that makes one upload serve every zoom.
///
/// One uploaded image, one scene, two placements side by side, one filter each. A device
/// that bound the sampler with the image, or that took the first command's decision for
/// the frame, draws the two halves alike and fails here while passing every test above.
#[test]
fn each_command_carries_its_own_filter_decision() {
    let mut device = device();
    let image = device.upload_image(&checkerboard()).expect("consistent");

    let mut builder = SceneBuilder::new();
    for (column, filter) in [
        (0.0, ImageFilter::Nearest),
        (SIDE as f32, ImageFilter::Linear),
    ] {
        builder
            .image(
                image,
                Affine::scale(SIDE as f32, SIDE as f32).then(Affine::translate(column, 0.0)),
                1.0,
                filter,
                None,
                BlendMode::Normal,
                None,
            )
            .expect("a finite placement at alpha 1");
    }
    let pixels = render(&mut device, &builder.finish(), SIDE * 2, SIDE);

    let level = |x: u32, y: u32| pixels[((y * SIDE * 2 + x) * 4) as usize];
    let left: Vec<u8> = (0..SIDE)
        .flat_map(|y| (0..SIDE).map(move |x| (x, y)))
        .map(|(x, y)| level(x, y))
        .collect();
    let right: Vec<u8> = (0..SIDE)
        .flat_map(|y| (0..SIDE).map(move |x| (x, y)))
        .map(|(x, y)| level(x + SIDE, y))
        .collect();

    for (at, &value) in left.iter().enumerate() {
        assert!(
            value == 0 || value == 255,
            "the command that asked for no interpolation was filtered anyway at {at}: \
             {left:?}"
        );
    }
    assert!(
        right.iter().any(|&value| value > 0 && value < 255),
        "the command that asked for interpolation was not filtered: {right:?}"
    );
}

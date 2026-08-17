//! A §7.10.5 program is **evaluated**, at every device pixel, at every zoom (ADR 0053).
//!
//! # The question, and whose it is
//!
//! The caller's hayro reading list carries #551 — a shading that a back end cannot express
//! is rasterised, and "the rasterisation is at a fixed low resolution regardless of the
//! output size". They name the general shape: *a paint the target cannot express is baked
//! at some resolution, and nothing tells the baker what resolution to use.* Their
//! `QUORRA_FUNCTION_PAINT.md` asked us not to bake at all, and ADR 0053 is the answer: a
//! type 1 shading's program is uploaded once, generated into a shader, and run per
//! fragment.
//!
//! The claim that follows is **resolution independence**, and it is not the same claim as
//! "the paint works". A grid baked at one scale and magnified draws a plausible picture at
//! every other scale; it is only wrong in the detail, which is exactly the failure mode
//! §5's "plausible-looking wrong page" names. So the gate has to be an assertion a baked
//! grid could not pass — a per-device-pixel value at three scales, and a discontinuity
//! whose device position is not a multiple of the zoom.
//!
//! # Where the expectations come from
//!
//! ISO 32000-2 §8.7.4.5.2 puts a type 1 shading's `Domain` in the shading's own space and
//! `Matrix` from there into the target space; §10.7.4 says the point a device pixel is
//! coloured from is its centre —
//!
//! > The position of the centre of such a pixel -in other words, the point whose
//! > coordinate values have fractional parts of one-half -shall be mapped back into source
//! > space
//!
//! — so the value at device column `i` under a viewport scale `s` is the program's value at
//! shading `x = (i + 0.5) / (CELL · s)`, and nothing about it depends on `s` except through
//! that quotient. The program is `x y 0.5` in `DeviceRGB`, so the red byte at that pixel is
//! `round(255 · x)` (ADR 0006's store). Every number below is that arithmetic, not a
//! previous run's output.
//!
//! # What this file does not test
//!
//! `tests/function_coverage.rs` already establishes that this paint's coverage is the
//! processor's under either `Coverage` setting, and why. Nothing here re-asks it.

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

use std::collections::BTreeSet;

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Compose, FillRule, FnOp, FnRange, FunctionId, Paint, Point, Rect, Scene,
    SceneBuilder, Segment,
};

mod function_support;

use function_support::programs::{DISCONTINUOUS, UNIT_RGB, unit_domain};

mod common;

use common::probe::pixel;

/// The target every frame is drawn into: [`CELL`] × the largest scale, so the whole shading
/// fits at every zoom and the comparison is over the same shading rather than over a
/// differently cropped one.
const SIZE: u32 = CELL * 4;

/// The shading's side in **scene** units. Small enough that a grid baked at scale 1 would
/// have thirty-two cells and its magnification at scale 4 would repeat each of them four
/// times — a difference of a hundred and twenty-eight assertions, not of one.
const CELL: u32 = 32;

/// The zooms every test sweeps. The caller's own corpus lane runs at 1 and 4.
const SCALES: [f32; 3] = [1.0, 2.0, 4.0];

/// `x y 0.5` in `DeviceRGB`: the two inputs a §8.7.4.5.2 shading pushes are already the
/// first two outputs, so one instruction leaves three values and the colour at a point *is*
/// its position in the shading's own space.
const POSITION_IS_COLOUR: [FnOp; 1] = [FnOp::PushReal(0.5)];

/// This file's adapter, named so a failure says where. ADR 0053 does not promise
/// cross-adapter identity for a generated shader, which is why the name is in the messages.
fn adapter_device() -> (Device, String) {
    let requested = std::env::var("QUORRA_ADAPTER").unwrap_or_else(|_| "llvmpipe".into());
    let device = Device::headless(&Options {
        adapter: Some(requested),
        ..Options::default()
    })
    .expect("the requested adapter is present");
    let name = device.description().to_string();
    (device, name)
}

fn rect_outline(rect: Rect) -> Vec<Segment> {
    vec![
        Segment::MoveTo(rect.min),
        Segment::LineTo(Point::new(rect.max.x, rect.min.y)),
        Segment::LineTo(rect.max),
        Segment::LineTo(Point::new(rect.min.x, rect.max.y)),
        Segment::Close,
    ]
}

/// A fill of the scene-space square `0..CELL` painted by `program` under `matrix`.
///
/// The scene is identical at every zoom — the viewport is the only thing that changes —
/// which is what makes the three frames "the same page at three magnifications" rather than
/// three different pages.
fn shaded_square(
    device: &mut Device,
    program: FunctionId,
    matrix: Affine,
    range: FnRange,
) -> Scene {
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(CELL as f32, CELL as f32),
        )))
        .expect("upload");
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Function {
                program,
                domain: unit_domain(),
                matrix,
                range,
                background: None,
            },
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("a valid function fill");
    builder.finish()
}

fn render_at(device: &mut Device, scene: &Scene, scale: f32) -> Vec<u8> {
    device
        .render(
            scene,
            &Viewport::full(SIZE, SIZE, Affine::scale(scale, scale)),
            Target::Readback,
        )
        .expect("the frame is inside every budget")
        .into_raster()
        .unwrap()
        .into_pixels()
}

/// #551's question answered in the strongest form available: at 1×, 2× and 4× the colour of
/// **every** device pixel is the program's value at that pixel's own centre.
///
/// A grid baked at any one of the three scales and magnified to the others would repeat a
/// value across the magnification factor; this assertion is made once per device pixel, so
/// a repeat is a failure at three quarters of them.
#[test]
fn every_device_pixel_carries_the_program_s_value_at_its_own_centre() {
    let (mut device, adapter) = adapter_device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("one instruction, no jumps");
    let scene = shaded_square(
        &mut device,
        program,
        Affine::scale(CELL as f32, CELL as f32),
        UNIT_RGB,
    );

    for scale in SCALES {
        let pixels = render_at(&mut device, &scene, scale);
        let span = (CELL as f32 * scale) as u32;
        let row = span / 2;
        for i in 0..span {
            // §10.7.4's pixel centre, mapped back through `Matrix` into the shading's own
            // space; §8.7.4.5.2's Domain is the unit square, so this is also the program's
            // input.
            let x = (i as f32 + 0.5) / (CELL as f32 * scale);
            let expected = (x * 255.0).round() as u8;
            let actual = pixel(&pixels, SIZE, i, row);
            assert!(
                i32::from(actual[0]).abs_diff(i32::from(expected)) <= 1,
                "{adapter}, scale {scale}, column {i}: red is {} where the program's value \
                 at that pixel's own centre is {expected}",
                actual[0]
            );
            assert_eq!(
                actual[3], 255,
                "scale {scale}, column {i}: the square is opaque"
            );
        }
    }
}

/// The same frames read the other way round, and the reason a baked grid cannot pass the
/// test above: **magnification adds detail here**.
///
/// The count of distinct red bytes across the shading is bounded by the number of points the
/// paint was evaluated at. Baked once at 1× and magnified, that count would be the same at
/// every scale; evaluated per fragment it grows with the zoom until the 8-bit store is the
/// only limit left.
#[test]
fn magnifying_the_page_adds_detail_rather_than_repeating_it() {
    let (mut device, adapter) = adapter_device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("one instruction, no jumps");
    let scene = shaded_square(
        &mut device,
        program,
        Affine::scale(CELL as f32, CELL as f32),
        UNIT_RGB,
    );

    let mut counts = Vec::new();
    for scale in SCALES {
        let pixels = render_at(&mut device, &scene, scale);
        let span = (CELL as f32 * scale) as u32;
        let row = span / 2;
        let distinct: BTreeSet<u8> = (0..span).map(|i| pixel(&pixels, SIZE, i, row)[0]).collect();
        counts.push(distinct.len() as u32);
    }
    assert_eq!(
        counts[0], CELL,
        "{adapter}: at scale 1 the shading spans {CELL} device pixels and each carries its \
         own value, so there are {CELL} of them; got {}",
        counts[0]
    );
    for (index, scale) in SCALES.iter().enumerate().skip(1) {
        let expected = (CELL as f32 * scale) as u32;
        assert_eq!(
            counts[index], expected,
            "{adapter}: at scale {scale} the shading spans {expected} device pixels, and a \
             paint that is evaluated gives each of them its own value; a grid baked at \
             scale 1 would still give {}",
            counts[0]
        );
    }
}

/// A discontinuity's device position is the decisive test, because it is the one number a
/// bake-then-magnify gets wrong *without* blurring anything.
///
/// `DISCONTINUOUS` is `x 0.5 ge { 1 0 0 } { 0 0 1 } ifelse`. The matrix below puts shading
/// `x = 0.5` at scene `x = 16.25` — deliberately **not** on the scene's integer grid — so
/// the step's device column is `ceil(16.25 · s − 0.5)`: 16, 32 and 65 at the three scales.
/// A grid baked at scale 1 and magnified would put it at 16, 32 and **64**, because the
/// step would have been rounded into a cell before the zoom existed. One pixel, and it is
/// the whole difference between evaluating and baking.
#[test]
fn a_discontinuity_lands_where_the_program_puts_it_and_not_on_the_zoom_s_grid() {
    let (mut device, adapter) = adapter_device();
    let program = device
        .upload_function(DISCONTINUOUS)
        .expect("a forward branch over three pushes each way");
    // Shading space → scene: the unit square onto `0.25 .. CELL + 0.25`.
    let matrix = Affine::scale(CELL as f32, CELL as f32).then(Affine::translate(0.25, 0.0));
    let scene = shaded_square(&mut device, program, matrix, UNIT_RGB);

    for scale in SCALES {
        let pixels = render_at(&mut device, &scene, scale);
        let row = (CELL as f32 * scale) as u32 / 2;
        // The first column whose centre is at or past shading x = 0.5.
        let step = (16.25_f32 * scale - 0.5).ceil() as u32;
        assert_eq!(
            pixel(&pixels, SIZE, step - 1, row),
            [0, 0, 255, 255],
            "{adapter}, scale {scale}: column {} is left of the step and must be the \
             program's second branch",
            step - 1
        );
        assert_eq!(
            pixel(&pixels, SIZE, step, row),
            [255, 0, 0, 255],
            "{adapter}, scale {scale}: column {step} is at the step and must be the \
             program's first branch — a grid baked before the zoom would still be blue here"
        );
    }
}

/// The control the two tests above need, and the one `doc/HANDOVER.md`'s newest trap asks
/// for: a claim that a paint is *not* baked is only as good as the proof that this fixture
/// reaches the function lane at all.
///
/// A `Paint::Function` whose program identifier the device never issued is refused by name.
/// If these scenes were quietly taking some other lane — a solid fallback, an unpainted
/// hole — the identifier would never be looked up and the refusal would not come.
#[test]
fn the_fixture_really_reaches_the_function_lane() {
    let (mut device, adapter) = adapter_device();
    let scene = shaded_square(
        &mut device,
        FunctionId(9999),
        Affine::scale(CELL as f32, CELL as f32),
        UNIT_RGB,
    );
    let refused = device.render(
        &scene,
        &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
        Target::Readback,
    );
    assert!(
        matches!(
            refused,
            Err(quorra_gpu::RenderError::UnknownFunction { .. })
        ),
        "{adapter}: this file's fixture must resolve a program identifier, or its \
         assertions are about something other than the function lane; got {refused:?}"
    );
}

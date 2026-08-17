//! A §7.10.5 program painted as one element of a **knockout** group (§11.4.6, ADR 0053).
//!
//! # Why this file exists
//!
//! `Style::of` hands the function lane the same erase/add pair every other lane draws a
//! knockout element with, `function_lane.wgsl`'s `fs_shape` is written for it, and
//! `pipeline::function` compiles all three styles from one module — and until this file
//! nothing drew one (`doc/notes-function-wiring.md` §4.5). A pipeline that compiles, is
//! selected, and is never drawn is a path that works until someone looks.
//!
//! # Where the expected values come from
//!
//! ISO 32000-2 §11.4.6 composites **every** element of a knockout group with the group's
//! *initial* backdrop rather than with the accumulated result, and takes a weighted average
//! of that with the immediate backdrop, weighted by the element's own source shape:
//!
//! > 𝛼gi = (1 − 𝑓si) × 𝛼gi−1 + 𝑓si × 𝛼t
//!
//! with the same weighting applied to colour. The group here is isolated, so the initial
//! backdrop is transparent (§11.4.5) and §11.3.6's compositing formula against `ab = 0`
//! leaves `co = as·Cs`, `ao = as` — the element's own premultiplied deposit. So the line
//! every test below is held against is
//!
//! ```text
//! P' = (1 − f) × P + S
//! ```
//!
//! per channel, premultiplied: `P` is the group holding everything before the element, `f`
//! is the element's **shape** (§11.6.4.2: geometry, so the coverage of the mark under its
//! clip) and `S` is the element's own premultiplied contribution. That is the same
//! statement `tests/knockout_blend.rs` holds the fill and the stroke arms to and the same
//! one ADR 0025 measured, and all four files that measure it now do so through one
//! arithmetic — `tests/common/clause.rs`, whose module comment carries the derivation.
//! What each file still derives for itself is which raster is `P`, which is `S` and where
//! `f` is read from, because that is the part that is about *its* lane.
//!
//! # The three things that are peculiar to *this* paint
//!
//! 1. **The paint is opaque where it marks and its colour varies per pixel**, so the wedge
//!    fixture — two diagonal edges, ADR 0025's own instrument, §4.1 of the brief's reason —
//!    measures the clause's line where a partially covered pixel makes the two readings
//!    disagree.
//! 2. **§8.7.4.5.2 leaves a point outside the transformed domain unpainted** when the
//!    shading has no `Background`, and an unpainted point has no shape: the knockout must
//!    leave the accumulated group exactly as it found it there. An erase pass that marked
//!    its whole quad — the obvious way to write `fs_shape`, and the one the shading lane's
//!    ramp would have got away with — knocks a hole in the page instead.
//! 3. **A `Background` whose alpha is below one has shape 1 at that opacity**, which is
//!    §11.4.7.2's distinction between the two, and a knockout group is the construction
//!    that can see it: it replaces the group's alpha rather than compositing over it.

// Test-file lint policy as in m1.rs; the reference math mirrors clause arithmetic.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    // Pixel indexing and the clause's own arithmetic, over rasters this file just drew.
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Device, Options};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, FnOp, FnRange, FunctionId, GroupSpec, OutlineId,
    Paint, Point, Rect, SceneBuilder, Segment,
};

mod common;

use common::clause::{deviation_from_the_clause, premul};
use common::headless::render;
use common::probe::pixel;

const SIZE: u32 = 64;

/// `x y 0.5` — the shortest program that is a function of both inputs and leaves three
/// values, so the colour at a point *is* its position. A knockout element whose deposit is
/// a constant would agree with a lane that mixed up which pixel it was shading.
const POSITION_IS_COLOUR: [FnOp; 1] = [FnOp::PushReal(0.5)];

/// The blend mode the first fixture gives its element: separable, and one whose
/// `B(Cb, Cs)` differs from `Cs` for the colours below, so taking §11.3.5 where §11.4.6
/// belongs is visible rather than a coincidence.
const MODE: BlendMode = BlendMode::Multiply;

/// The opaque content the group holds before the element. Its immediate backdrop is
/// therefore opaque while the group's initial backdrop is still transparent, which is the
/// whole difference between the two readings.
const UNDER: Color = Color {
    r: 0.9,
    g: 0.2,
    b: 0.1,
    a: 1.0,
};

/// The device this file's assertions are made on, named so a failure says where: ADR 0053
/// promises **no** cross-adapter identity for this paint, so every message carries the
/// adapter. `QUORRA_ADAPTER` picks another; the suite's default is the software
/// rasteriser, as everywhere else in this tree.
fn device() -> (Device, String) {
    let requested = std::env::var("QUORRA_ADAPTER").unwrap_or_else(|_| "llvmpipe".into());
    let device = Device::headless(&Options {
        adapter: Some(requested),
        ..Options::default()
    })
    .expect("the requested adapter is present");
    let name = device.description().to_string();
    (device, name)
}

/// A triangle with two diagonal edges, so partially covered pixels exist: axis-aligned
/// rectangles would agree while being wrong (§4.1 of the brief). It is also what sends the
/// fill through the **rasterised coverage** lane rather than the rect-hinted one.
fn wedge(device: &mut Device) -> OutlineId {
    device
        .upload_outline(&[
            Segment::MoveTo(Point::new(10.0, 10.0)),
            Segment::LineTo(Point::new(54.0, 10.0)),
            Segment::LineTo(Point::new(10.0, 54.0)),
            Segment::Close,
        ])
        .unwrap()
}

/// The whole target as a closed rectangle: the **rect-hinted** lane, where the quad is the
/// shape's device rectangle rather than a scratch tile. Both lanes place the quad
/// differently, so both are drawn under knockout here.
fn full_rect(device: &mut Device) -> OutlineId {
    let side = SIZE as f32;
    device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(side, 0.0)),
            Segment::LineTo(Point::new(side, side)),
            Segment::LineTo(Point::new(0.0, side)),
            Segment::Close,
        ])
        .unwrap()
}

/// An isolated group, knocking out or not — the two readings, written as one difference.
fn group(knockout: bool) -> GroupSpec {
    GroupSpec {
        alpha: 1.0,
        blend: BlendMode::Normal,
        clip: None,
        knockout,
        mask: None,
        isolated: true,
        compose: Compose::SrcOver,
    }
}

/// The opaque cover the group holds before the element: full-target, so compositing the
/// finished group over the empty page is a copy and what a probe reads is the group's own
/// arithmetic.
fn cover(builder: &mut SceneBuilder) {
    builder
        .rect(
            Rect::new(Point::new(0.0, 0.0), Point::new(SIZE as f32, SIZE as f32)),
            Affine::IDENTITY,
            UNDER,
            None,
            None,
        )
        .unwrap();
}

/// One function-painted fill: the domain rectangle mapped onto the whole target by
/// §8.7.4.5.2's `Matrix`, so the value at a pixel is its own position in the target.
fn function_fill(
    builder: &mut SceneBuilder,
    outline: OutlineId,
    program: FunctionId,
    domain: Rect,
    background: Option<Color>,
    blend: BlendMode,
) {
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Function {
                program,
                domain,
                matrix: Affine::scale(SIZE as f32, SIZE as f32),
                range: FnRange::Rgb([[0.0, 1.0]; 3]),
                background,
            },
            None,
            blend,
            Compose::SrcOver,
            None,
        )
        .expect("a valid function fill");
}

/// The unit square: the domain that covers every point the `Matrix` maps into the target.
fn unit_square() -> Rect {
    Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0))
}

/// The left quarter of the unit square, so three quarters of a full-target shape falls
/// outside the transformed domain rectangle (§8.7.4.5.2).
fn left_quarter() -> Rect {
    Rect::new(Point::new(0.0, 0.0), Point::new(0.25, 1.0))
}

/// **The clause's line, on the lane that had never drawn it.**
///
/// The element is a function-painted fill of the wedge, blended `Multiply`, inside a
/// knockout group over an opaque cover. §11.4.6 replaces a coverage-fraction of the
/// accumulated group with the element composited against the group's *initial* backdrop —
/// where §11.3.6 keeps only `co = as·Cs` and the blend function disappears — so the
/// expected value is `(1 − f) × P + S` and carries no trace of `Multiply`.
///
/// The same element in an **ordinary** group is measured against the same line and must
/// miss it by a wide margin: a fixture where the two readings agree holds nothing.
///
/// # What this test does *not* hold, and which one does
///
/// The element here is opaque wherever it marks, and for an opaque source §11.4.6's
/// replacement and an ordinary premultiplied over-composite are the same arithmetic —
/// `(1 − f)P + f·Cs` either way. So this fixture separates §11.4.6 from §11.3.5's implicit
/// blend group, which is what it is for, and it would **not** notice a lane that drew one
/// `Over` pass where ADR 0010's erase/add pair belongs. Verified by forcing exactly that,
/// which leaves this test and the domain test green and fails
/// [`a_background_alpha_knocks_out_at_full_shape`] — that one is where the pair is held,
/// because a source of alpha below one is the only thing the two arithmetics disagree
/// about.
#[test]
fn a_function_fill_inside_a_knockout_group_takes_the_replacement() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("`x y 0.5` is admitted");
    let outline = wedge(&mut device);

    // P: what the group holds before the element.
    let mut before = SceneBuilder::new();
    before
        .group(group(true), |body| {
            cover(body);
            Ok(())
        })
        .unwrap();
    let before = render(&mut device, &before.finish(), SIZE, SIZE);

    // S, and the shape f that weights it, read from the device rather than assumed. The
    // deposit is drawn under Normal because that is what §11.3.6 leaves of any mode against
    // the transparent initial backdrop.
    let mut alone = SceneBuilder::new();
    function_fill(
        &mut alone,
        outline,
        program,
        unit_square(),
        None,
        BlendMode::Normal,
    );
    let deposit = render(&mut device, &alone.finish(), SIZE, SIZE);

    // The shape of the same mark drawn opaquely. The function paint is opaque wherever it
    // marks — §8.7.4.5.2's domain covers the whole shape here and the generated shader
    // returns alpha 1 inside it — so the deposit's own alpha must already *be* the shape,
    // and that is asserted rather than assumed: it is the premise the expectation rests on.
    let mut opaque = SceneBuilder::new();
    opaque
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Solid(Color::new(1.0, 1.0, 1.0, 1.0)),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    let shape = render(&mut device, &opaque.finish(), SIZE, SIZE);
    let worst_alpha = shape
        .iter()
        .skip(3)
        .step_by(4)
        .zip(deposit.iter().skip(3).step_by(4))
        .map(|(a, b)| i32::from(*a).abs_diff(i32::from(*b)))
        .max()
        .unwrap_or(0);
    assert!(
        worst_alpha <= 1,
        "{adapter}: inside its domain the function paint is opaque, so its alpha is the \
         mark's shape; worst difference {worst_alpha}"
    );

    let in_group = |device: &mut Device, knockout: bool| {
        let mut builder = SceneBuilder::new();
        builder
            .group(group(knockout), |body| {
                cover(body);
                function_fill(body, outline, program, unit_square(), None, MODE);
                Ok(())
            })
            .unwrap();
        render(device, &builder.finish(), SIZE, SIZE)
    };
    let knocked = in_group(&mut device, true);
    let ordinary = in_group(&mut device, false);

    let (worst_knockout, partial) = deviation_from_the_clause(&before, &shape, &deposit, &knocked);
    let (worst_ordinary, _) = deviation_from_the_clause(&before, &shape, &deposit, &ordinary);
    eprintln!(
        "{adapter}: worst knockout {worst_knockout:.2}, worst ordinary group {worst_ordinary:.2}"
    );
    assert!(
        partial > 30,
        "{adapter}: the fixture must have partially covered pixels for this to mean \
         anything: {partial}"
    );
    assert!(
        worst_knockout <= 3.0,
        "{adapter}: a blended function fill inside a knockout group must be §11.4.6's \
         replacement — its element composites with the transparent initial backdrop, where \
         §11.3.6 leaves nothing of the blend function. Worst premultiplied deviation \
         {worst_knockout}"
    );
    assert!(
        worst_ordinary >= 16.0,
        "{adapter}: and §11.3.5's implicit one-element group must not be — a fixture where \
         the two agree holds nothing: {worst_ordinary}"
    );
}

/// **A point the clause leaves unpainted has no shape, so it knocks nothing out.**
///
/// ISO 32000-2 §8.7.4.5.2:
///
/// > Points within the shading's bounding box (BBox) that fall outside this transformed
/// > domain rectangle shall be painted with the shading's background colour (Background);
/// > if the shading dictionary has no Background entry, such points shall be left
/// > unpainted.
///
/// §11.4.6 weights the replacement by the element's shape and §11.6.4.2 makes that the
/// object's geometry; an unpainted point is not part of it. So over three quarters of this
/// full-target fill the group must hold exactly what it held before the element — which is
/// the assertion an erase pass marking its whole quad fails, and it fails it by knocking a
/// transparent hole through an opaque page rather than by a shade of colour.
///
/// The fill is a rectangle, so this draws the **rect-hinted** lane, where the first test
/// draws the rasterised-coverage one.
#[test]
fn a_point_the_domain_leaves_unpainted_knocks_nothing_out() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let outline = full_rect(&mut device);

    let mut before = SceneBuilder::new();
    before
        .group(group(true), |body| {
            cover(body);
            Ok(())
        })
        .unwrap();
    let before = render(&mut device, &before.finish(), SIZE, SIZE);

    let mut builder = SceneBuilder::new();
    builder
        .group(group(true), |body| {
            cover(body);
            function_fill(
                body,
                outline,
                program,
                left_quarter(),
                None,
                BlendMode::Normal,
            );
            Ok(())
        })
        .unwrap();
    let knocked = render(&mut device, &builder.finish(), SIZE, SIZE);

    // x < 16 is inside the transformed domain rectangle; x >= 16 is outside it.
    for x in [20_u32, 32, 48, 63] {
        assert_eq!(
            pixel(&knocked, SIZE, x, 32),
            pixel(&before, SIZE, x, 32),
            "{adapter}: at x = {x} the clause leaves the point unpainted, so the knockout \
             element has no shape there and the group keeps what it accumulated"
        );
    }
    // And the element really did draw, so the test above cannot be passing because nothing
    // reached the target: inside the domain the group carries the function's own value at
    // the pixel's centre (§10.7.4), which at f = 1 is §11.4.6's replacement in full.
    for (x, want) in [(2_u32, 2.5_f32), (8, 8.5)] {
        let got = pixel(&knocked, SIZE, x, 32);
        let expected = (want / SIZE as f32 * 255.0).round() as i32;
        assert!(
            (i32::from(got[0]) - expected).abs() <= 1,
            "{adapter}: inside the domain the accumulated cover is replaced by the \
             element's own colour; expected red {expected} ± 1, got {got:?}"
        );
        assert_eq!(
            got[3], 255,
            "{adapter}: and the replacement is opaque, because the element is"
        );
    }
}

/// **A `Background` of alpha ½ has shape 1 at opacity ½** — §11.4.7.2's distinction, on the
/// one construction that can observe it.
///
/// > The existence of the knockout feature is the main reason for maintaining a separate
/// > shape value rather than only a single alpha that combines shape and opacity.
///
/// (§11.4.6, quoted by ADR 0025.) Outside the transformed domain the clause paints the
/// background, so the point *is* marked: `f = 1`, and §11.4.6's average keeps none of the
/// accumulated group — `P' = S`, the background's own premultiplied colour, at alpha ½.
/// The same element in an ordinary group composites by §11.3.6 instead and leaves half the
/// opaque cover behind, at alpha 1. The two answers differ in every channel, which is what
/// makes this a test of shape rather than of alpha.
///
/// **This is the test that holds ADR 0010's erase/add pair for this lane.** The other two
/// fixtures paint an opaque source, where the replacement and an over-composite are the
/// same arithmetic; a source of alpha ½ at shape 1 is what makes the pair observable, and
/// forcing the lane to a single `Over` pass fails this test alone.
#[test]
fn a_background_alpha_knocks_out_at_full_shape() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let outline = full_rect(&mut device);
    // 0.2, 0.4 and 0.6 are exact in 8 bits (51, 102, 153); the alpha is the quantity under
    // test and is stated as a fraction.
    let background = Color::new(0.2, 0.4, 0.6, 0.5);

    let in_group = |device: &mut Device, knockout: bool| {
        let mut builder = SceneBuilder::new();
        builder
            .group(group(knockout), |body| {
                cover(body);
                function_fill(
                    body,
                    outline,
                    program,
                    left_quarter(),
                    Some(background),
                    BlendMode::Normal,
                );
                Ok(())
            })
            .unwrap();
        render(device, &builder.finish(), SIZE, SIZE)
    };
    let knocked = in_group(&mut device, true);
    let ordinary = in_group(&mut device, false);

    // §11.4.6 with f = 1: the group holds the element composited against its transparent
    // initial backdrop, which §11.3.6 reduces to `co = as·Cs`, `ao = as`.
    let expected = [
        background.r * background.a,
        background.g * background.a,
        background.b * background.a,
        background.a,
    ];
    // §11.3.6 with as = ½ over an opaque backdrop, for the control: co = as·Cs + ab·(1 −
    // as)·Cb, and the blend term drops out because the mode is Normal.
    let (source, backdrop) = (background.a, 1.0 - background.a);
    let composited = [
        source * background.r + backdrop * UNDER.r,
        source * background.g + backdrop * UNDER.g,
        source * background.b + backdrop * UNDER.b,
        1.0,
    ];

    for x in [32_u32, 48, 63] {
        let at = ((32 * SIZE + x) * 4) as usize;
        for channel in 0..3 {
            let got = premul(&knocked, at, channel) / 255.0;
            assert!(
                (got - expected[channel]).abs() <= 3.0 / 255.0,
                "{adapter}: at x = {x} channel {channel}, §11.4.6 replaces the group with \
                 the background's own premultiplied colour: expected {}, got {got}",
                expected[channel]
            );
            let control = premul(&ordinary, at, channel) / 255.0;
            assert!(
                (control - composited[channel]).abs() <= 3.0 / 255.0,
                "{adapter}: and an ordinary group composites it by §11.3.6 instead: \
                 expected {}, got {control}",
                composited[channel]
            );
        }
        let alpha = f32::from(pixel(&knocked, SIZE, x, 32)[3]) / 255.0;
        assert!(
            (alpha - expected[3]).abs() <= 3.0 / 255.0,
            "{adapter}: at x = {x} the knockout replaces the group's alpha with the \
             element's own — shape 1, opacity ½ — rather than compositing over it: \
             expected {}, got {alpha}",
            expected[3]
        );
        assert_eq!(
            pixel(&ordinary, SIZE, x, 32)[3],
            255,
            "{adapter}: where the ordinary group keeps the opaque cover underneath"
        );
    }
}

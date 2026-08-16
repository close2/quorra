//! A blended element inside a knockout group takes §11.4.6, not §11.3.5.
//!
//! # Where the expected values come from
//!
//! ISO 32000-2 §11.4.6 composites **every** element of a knockout group with the group's
//! *initial* backdrop rather than with the accumulated result, and takes a weighted
//! average of that with the immediate backdrop, weighted by the element's own source
//! shape:
//!
//! > 𝛼gi = (1 − 𝑓si) × 𝛼gi−1 + 𝑓si × 𝛼t
//!
//! with the same weighting applied to colour. In an isolated group the initial backdrop
//! is transparent (§11.4.5), so `𝛼t` and the colour beside it are the element composited
//! against `ab = 0` — and there §11.3.6's compositing formula, premultiplied,
//!
//! ```text
//! co = as·(1 − ab)·Cs + ab·(1 − as)·Cb + as·ab·B(Cb, Cs)
//! ao = as + ab·(1 − as)
//! ```
//!
//! loses both terms carrying `B`: `co = as·Cs`, `ao = as`. **Every blend mode degenerates
//! to Normal against the transparent initial backdrop**, which is what
//! [`every_mode_degenerates_against_the_transparent_initial_backdrop`] holds directly, and
//! what lets the other two tests read the deposit `S` off a Normal-blended draw.
//!
//! So the expectation for one element of a knockout group is `P' = (1 − f) × P + S` per
//! channel, premultiplied — the same line ADR 0032's fixture measures — where `P` is the
//! group holding everything *before* the element, `f` is the element's shape (§11.6.4.2:
//! geometry, so the alpha of the same element drawn opaquely onto transparency) and `S` is
//! the element's own premultiplied contribution.
//!
//! The other reading — §11.3.5's implicit one-element group, composited by §11.3.6 over
//! the accumulated group content — is what an *ordinary* group does with the same element,
//! and each test measures it too: a fixture where the two agree would hold nothing.
//! `encode_stroke` took that reading inside a knockout group until 2026-08-14, where
//! `encode_fill` and `encode_image` had always taken this one.

// Test-file lint policy as in m1.rs; the reference math mirrors clause arithmetic.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    // Pixel indexing and the clause's own arithmetic, over rasters this file just drew.
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Device, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, GroupSpec, LineCap, LineJoin, OutlineId, Paint,
    Point, Rect, Scene, SceneBuilder, SceneError, Segment, Stroke,
};

mod common;

use common::clause::deviation_from_the_clause;
use common::headless::device;

const SIZE: u32 = 64;

/// The blend mode every fixture here uses: separable, and — the property that makes it
/// discriminating — one whose `B(Cb, Cs)` differs from `Cs` for the colours below, so
/// taking §11.3.5 where §11.4.6 belongs is visible rather than a coincidence.
const MODE: BlendMode = BlendMode::Multiply;

/// The opaque content the element lands on inside the group. Its immediate backdrop is
/// therefore opaque, and the group's initial backdrop still transparent — which is the
/// whole difference between the two readings.
const UNDER: Color = Color {
    r: 0.9,
    g: 0.2,
    b: 0.1,
    a: 1.0,
};

/// The element's paint. Half-opaque, so shape and opacity are different numbers and
/// §11.4.6's replacement differs from an ordinary composite even under Normal.
const OBJECT: Color = Color {
    r: 0.1,
    g: 0.4,
    b: 0.9,
    a: 0.5,
};

fn render(device: &mut Device, scene: &Scene) -> Vec<u8> {
    device
        .render(
            scene,
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders")
        .into_raster()
        .unwrap()
        .into_pixels()
}

/// A triangle with two diagonal edges, so partially covered pixels exist: axis-aligned
/// rectangles would agree while being wrong (§4.1 of the brief).
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

/// The opaque cover the group holds before the element: full-target, so the group's
/// buffer is opaque under the element and compositing the finished group over the empty
/// page is a copy — what the probe reads is the group's own arithmetic.
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

/// One element of a group, in the lane under test, at a stated colour and blend mode. The
/// two implementations below are the fill arm and the stroke arm of the encoder.
type Element = fn(&mut SceneBuilder, OutlineId, Color, BlendMode) -> Result<(), SceneError>;

fn filled(
    builder: &mut SceneBuilder,
    outline: OutlineId,
    colour: Color,
    blend: BlendMode,
) -> Result<(), SceneError> {
    builder.fill(
        outline,
        Affine::IDENTITY,
        FillRule::NonZero,
        Paint::Solid(colour),
        None,
        blend,
        Compose::SrcOver,
        None,
    )
}

fn stroked(
    builder: &mut SceneBuilder,
    outline: OutlineId,
    colour: Color,
    blend: BlendMode,
) -> Result<(), SceneError> {
    builder.stroke(
        outline,
        Affine::IDENTITY,
        Stroke {
            // Wide enough that the band holds interior pixels of its own, so the
            // discriminating case is not only the antialiased edge.
            width: 9.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 4.0,
        },
        Paint::Solid(colour),
        None,
        blend,
        None,
    )
}

/// The measurement both lane tests make: the element inside a knockout group against
/// §11.4.6's line, and the same element inside an ordinary group — §11.3.5's implicit
/// one-element group composited over the accumulated content — against the same line.
///
/// Returns `(knockout deviation, ordinary deviation, partially covered pixels)`.
fn measure(device: &mut Device, outline: OutlineId, element: Element) -> (f32, f32, u32) {
    // P: what the group holds before the element.
    let mut before = SceneBuilder::new();
    before
        .group(group(true), |body| {
            cover(body);
            Ok(())
        })
        .unwrap();
    let before = render(device, &before.finish());

    // f and S, read from the device rather than assumed. The deposit is drawn under
    // Normal because that is what §11.3.6 leaves of any mode against `ab = 0`.
    let onto_transparency = |device: &mut Device, colour: Color| {
        let mut builder = SceneBuilder::new();
        element(&mut builder, outline, colour, BlendMode::Normal).unwrap();
        render(device, &builder.finish())
    };
    let shape = onto_transparency(device, Color::new(1.0, 1.0, 1.0, 1.0));
    let deposit = onto_transparency(device, OBJECT);

    let in_group = |device: &mut Device, knockout: bool| {
        let mut builder = SceneBuilder::new();
        builder
            .group(group(knockout), |body| {
                cover(body);
                element(body, outline, OBJECT, MODE)
            })
            .unwrap();
        render(device, &builder.finish())
    };
    let knocked = in_group(device, true);
    let ordinary = in_group(device, false);

    let (worst_knockout, partial) = deviation_from_the_clause(&before, &shape, &deposit, &knocked);
    let (worst_ordinary, _) = deviation_from_the_clause(&before, &shape, &deposit, &ordinary);
    (worst_knockout, worst_ordinary, partial)
}

/// The two assertions every lane owes the clause, so the two lane tests differ only in
/// which arm of the encoder they name.
fn assert_takes_the_replacement(lane: &str, (knockout, ordinary, partial): (f32, f32, u32)) {
    eprintln!("{lane}: worst knockout {knockout:.2}, worst ordinary group {ordinary:.2}");
    assert!(
        partial > 30,
        "the fixture must have partially covered pixels for this to mean anything: {partial}"
    );
    assert!(
        knockout <= 3.0,
        "a blended {lane} inside a knockout group must be §11.4.6's replacement — its \
         element composites with the transparent initial backdrop, where §11.3.6 leaves \
         nothing of the blend function. Worst premultiplied deviation {knockout}"
    );
    assert!(
        ordinary >= 16.0,
        "and §11.3.5's implicit one-element group must not be — a fixture where the two \
         agree holds nothing: {ordinary}"
    );
}

/// The premise the other two tests rest on, held on its own: against a transparent
/// backdrop §11.3.6 keeps only `co = as·Cs`, so a mode is not observable there.
#[test]
fn every_mode_degenerates_against_the_transparent_initial_backdrop() {
    let mut device = device();
    let outline = wedge(&mut device);

    for element in [filled as Element, stroked as Element] {
        let onto_transparency = |device: &mut Device, blend: BlendMode| {
            let mut builder = SceneBuilder::new();
            element(&mut builder, outline, OBJECT, blend).unwrap();
            render(device, &builder.finish())
        };
        let normal = onto_transparency(&mut device, BlendMode::Normal);
        for mode in BlendMode::ALL {
            let blended = onto_transparency(&mut device, mode);
            let worst = normal
                .iter()
                .zip(&blended)
                .map(|(a, b)| i32::from(*a).abs_diff(i32::from(*b)))
                .max()
                .unwrap_or(0);
            assert!(
                worst <= 1,
                "§11.3.6 with ab = 0 leaves co = as·Cs, so {mode:?} may not be visible \
                 against transparency; worst channel difference {worst}"
            );
        }
    }
}

/// **The defect this file was written for.** A blended stroke inside a knockout group is
/// §11.4.6's replacement, not §11.3.5's implicit group composited over the accumulation.
///
/// `encode_stroke` wrapped the stroke on the blend mode alone, where `encode_fill` and
/// `encode_image` also required the enclosing style to be `Over`; the wrap is the reading
/// this test refuses.
#[test]
fn a_blended_stroke_inside_a_knockout_group_takes_the_replacement() {
    let mut device = device();
    let outline = wedge(&mut device);
    let measured = measure(&mut device, outline, stroked);
    assert_takes_the_replacement("stroke", measured);
}

/// The same clause on the fill arm, which has always taken it — here so the two arms are
/// held to one statement of §11.4.6 rather than to two, and so a later change to either
/// has to move both.
#[test]
fn a_blended_fill_inside_a_knockout_group_takes_the_replacement() {
    let mut device = device();
    let outline = wedge(&mut device);
    let measured = measure(&mut device, outline, filled);
    assert_takes_the_replacement("fill", measured);
}

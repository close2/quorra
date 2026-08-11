//! §11.4.6's two stages, asked for by name: `Compose::DestOut` then `Compose::Plus`.
//!
//! ADR 0025, from the caller's feedback §14. `Compose::Src` reads an element's **shape**
//! off the alpha it is drawn with, which is right for the half of §11.4.6 where they are
//! the same quantity and wrong for the other half — a nested group, or an element under a
//! soft mask, where §11.6.4.2 gives shape from geometry while §11.6.4.3's mask and
//! §11.6.4.4's constant alpha are opacity. The clause's own sentence:
//!
//! > The existence of the knockout feature is the main reason for maintaining a separate
//! > shape value rather than only a single alpha that combines shape and opacity.
//!
//! With the two operators a caller writes the stage as `P' = (1 − f) × P + S` in one mark
//! each. What this file holds is that the pair *is* that formula, and — the part that
//! decides whether the pair was worth adding — that **source-over is not**: it weights
//! the backdrop a second time, by `1 − shape × opacity` where the clause weights it by
//! `1 − shape` alone. The two agree wherever the object is opaque or the shape is 0 or 1,
//! so the discriminating case is a half-covered pixel under a half-opaque mark, which the
//! caller pins at 32 of 255.

// Test-file lint policy as in m1.rs; the reference math mirrors clause arithmetic.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, GroupSpec, OutlineId, Paint, Point, Scene,
    SceneBuilder, SceneError, Segment, StagedComposeReason,
};

const SIZE: u32 = 64;

fn device() -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

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

/// A triangle with a diagonal edge, so half-covered pixels exist: axis-aligned
/// rectangles would agree while being wrong, which is the trap `Compose`'s own doc
/// comment names.
fn wedge(device: &mut Device) -> OutlineId {
    device
        .upload_outline(&[
            Segment::MoveTo(Point::new(8.0, 8.0)),
            Segment::LineTo(Point::new(56.0, 8.0)),
            Segment::LineTo(Point::new(8.0, 56.0)),
            Segment::Close,
        ])
        .unwrap()
}

fn fill(
    builder: &mut SceneBuilder,
    outline: OutlineId,
    colour: Color,
    compose: Compose,
) -> Result<(), SceneError> {
    builder.fill(
        outline,
        Affine::IDENTITY,
        FillRule::NonZero,
        Paint::Solid(colour),
        None,
        BlendMode::Normal,
        compose,
        None,
    )
}

/// The backdrop every scene here starts from.
fn backdrop(builder: &mut SceneBuilder) {
    builder
        .rect(
            quorra_scene::Rect::new(Point::new(0.0, 0.0), Point::new(SIZE as f32, SIZE as f32)),
            Affine::IDENTITY,
            Color::new(0.9, 0.2, 0.1, 1.0),
            None,
            None,
        )
        .unwrap();
}

/// The pair draws §11.4.6's second stage, and source-over does not.
///
/// Every quantity in the clause's line is read from the device rather than assumed:
/// the **shape** `f` is the alpha of the shape-only object drawn onto transparency, and
/// the **deposit** `S` is the object drawn onto transparency, which is its own
/// premultiplied contribution. The expectation is then `P' = (1 − f) × P + S`, per
/// channel, in premultiplied form.
#[test]
fn the_pair_is_the_clause_and_source_over_is_not() {
    let mut device = device();
    let outline = wedge(&mut device);
    // Half-opaque, so opacity and shape are different numbers — the case where the two
    // ways of writing the stage disagree.
    let object = Color::new(0.1, 0.4, 0.9, 0.5);

    let onto_transparency = |device: &mut Device, colour: Color| {
        let mut builder = SceneBuilder::new();
        fill(&mut builder, outline, colour, Compose::SrcOver).unwrap();
        render(device, &builder.finish())
    };
    let shape = onto_transparency(&mut device, Color::new(1.0, 1.0, 1.0, 1.0));
    let deposit = onto_transparency(&mut device, object);

    let mut plain = SceneBuilder::new();
    backdrop(&mut plain);
    let plain = render(&mut device, &plain.finish());

    let mut staged_scene = SceneBuilder::new();
    backdrop(&mut staged_scene);
    fill(
        &mut staged_scene,
        outline,
        Color::new(0.0, 0.0, 0.0, 1.0),
        Compose::DestOut,
    )
    .unwrap();
    fill(&mut staged_scene, outline, object, Compose::Plus).unwrap();
    let staged = render(&mut device, &staged_scene.finish());

    let mut over_scene = SceneBuilder::new();
    backdrop(&mut over_scene);
    fill(&mut over_scene, outline, object, Compose::SrcOver).unwrap();
    let over = render(&mut device, &over_scene.finish());

    let premul = |raster: &[u8], at: usize, channel: usize| {
        f32::from(raster[at + channel]) * f32::from(raster[at + 3]) / 255.0
    };

    let (mut worst_staged, mut worst_over) = (0.0_f32, 0.0_f32);
    let mut partial_pixels = 0_u32;
    for pixel in 0..(SIZE * SIZE) as usize {
        let at = pixel * 4;
        let f = f32::from(shape[at + 3]) / 255.0;
        if f > 0.0 && f < 1.0 {
            partial_pixels += 1;
        }
        for channel in 0..3 {
            // P' = (1 − f) × P + S, premultiplied (§11.4.6's second stage).
            let expected =
                (1.0 - f).mul_add(premul(&plain, at, channel), premul(&deposit, at, channel));
            worst_staged = worst_staged.max((premul(&staged, at, channel) - expected).abs());
            worst_over = worst_over.max((premul(&over, at, channel) - expected).abs());
        }
    }

    eprintln!("worst staged {worst_staged:.2}, worst source-over {worst_over:.2}");
    assert!(
        partial_pixels > 30,
        "the fixture must have partially covered pixels for this to mean anything: {partial_pixels}"
    );
    assert!(
        worst_staged <= 3.0,
        "the staged pair must be §11.4.6's line; worst premultiplied deviation {worst_staged}"
    );
    assert!(
        worst_over >= 16.0,
        "and source-over must not be — it weights the backdrop by 1 − shape × opacity \
         where the clause weights it by 1 − shape. Worst deviation {worst_over}, which \
         is the residue this pair exists to remove"
    );
}

/// `DestOut` weights by **shape**, not by the paint's alpha: the same mark drawn with a
/// half-opaque paint erases exactly as much as an opaque one, which is the distinction
/// the operator exists for.
#[test]
fn dest_out_weights_by_shape_and_not_by_opacity() {
    let mut device = device();
    let outline = wedge(&mut device);

    let erase_with = |device: &mut Device, alpha: f32| {
        let mut builder = SceneBuilder::new();
        backdrop(&mut builder);
        fill(
            &mut builder,
            outline,
            Color::new(0.0, 0.0, 0.0, alpha),
            Compose::DestOut,
        )
        .unwrap();
        render(device, &builder.finish())
    };

    let opaque = erase_with(&mut device, 1.0);
    let translucent = erase_with(&mut device, 0.25);
    assert_eq!(
        opaque, translucent,
        "§11.6.4.2's shape comes from geometry; the paint's alpha is §11.6.4.4's \
         opacity and may not change what is erased"
    );

    // And it erases: the middle of the wedge is gone, the outside is untouched.
    let inside = ((20 * SIZE + 20) * 4) as usize;
    let outside = ((60 * SIZE + 60) * 4) as usize;
    assert_eq!(
        opaque[inside + 3],
        0,
        "inside the shape the backdrop is erased"
    );
    assert_eq!(
        opaque[outside + 3],
        255,
        "outside it nothing is touched — the property that makes this operator safe \
         where a bounding-box composite is not"
    );
}

/// The two positions that already stage §11.4.6 refuse a staged mark rather than
/// applying the clause twice.
#[test]
fn the_two_positions_that_already_stage_the_clause_refuse() {
    let mut device = device();
    let outline = wedge(&mut device);
    let colour = Color::new(0.2, 0.2, 0.2, 1.0);

    // A blend mode wraps the mark in an implicit one-element group (§11.3.5).
    let mut builder = SceneBuilder::new();
    let error = builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Solid(colour),
            None,
            BlendMode::Multiply,
            Compose::Plus,
            None,
        )
        .expect_err("a blended staged mark composes the group, not the element");
    assert_eq!(
        error,
        SceneError::StagedComposeUnsupported {
            compose: Compose::Plus,
            reason: StagedComposeReason::BlendNotNormal,
        }
    );

    // Inside a knockout group, every element is already staged per §11.4.6.
    let mut builder = SceneBuilder::new();
    let error = builder
        .group(
            GroupSpec {
                alpha: 1.0,
                blend: BlendMode::Normal,
                clip: None,
                knockout: true,
                mask: None,
                isolated: true,
            },
            |body| fill(body, outline, colour, Compose::DestOut),
        )
        .expect_err("a knockout group already stages the clause for its elements");
    assert_eq!(
        error,
        SceneError::StagedComposeUnsupported {
            compose: Compose::DestOut,
            reason: StagedComposeReason::InsideKnockoutGroup,
        }
    );

    // And the ordinary operators are unaffected in both positions.
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Solid(colour),
            None,
            BlendMode::Multiply,
            Compose::Src,
            None,
        )
        .expect("Compose::Src under a blend mode is untouched by ADR 0025");
}

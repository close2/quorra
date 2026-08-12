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
    clippy::cast_precision_loss,
    // Pixel indexing and the clause's own arithmetic, over rasters this file just drew.
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, GroupComposeReason, GroupSpec, MaskKind,
    OutlineId, Paint, Point, Scene, SceneBuilder, SceneError, Segment, StagedComposeReason,
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

    // And inside a knockout group it is *accepted*, which is where §11.4.6 puts it
    // (ADR 0032). `the_pair_inside_a_knockout_group_is_the_clause` draws it.
    let mut builder = SceneBuilder::new();
    builder
        .group(
            GroupSpec {
                alpha: 1.0,
                blend: BlendMode::Normal,
                clip: None,
                knockout: true,
                mask: None,
                isolated: true,
                compose: Compose::SrcOver,
            },
            |body| fill(body, outline, colour, Compose::DestOut),
        )
        .expect("§11.4.6 weights each element by its own shape, staged or not");

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

/// A half-transparent alpha soft mask over the whole target.
///
/// What makes the knockout fixture discriminating: §11.6.4.3's mask is *opacity* and
/// §11.6.4.2's shape is geometry, so a masked element's shape is its outline and its alpha
/// is half of it. An element whose shape *is* its coverage — an ordinary solid fill — is
/// staged correctly by a knockout group on its own, and a fixture built from one would
/// pass whatever ADR 0032 decided.
fn half(builder: &mut SceneBuilder) -> Result<quorra_scene::MaskId, SceneError> {
    builder.mask(MaskKind::Alpha, None, |body| {
        body.rect(
            quorra_scene::Rect::new(Point::new(0.0, 0.0), Point::new(SIZE as f32, SIZE as f32)),
            Affine::IDENTITY,
            Color::new(1.0, 1.0, 1.0, 0.5),
            None,
            None,
        )
    })
}

/// The wedge under that mask, composed as asked.
fn masked(
    builder: &mut SceneBuilder,
    outline: OutlineId,
    colour: Color,
    mask: quorra_scene::MaskId,
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
        Some(mask),
    )
}

/// The worst premultiplied deviation from §11.4.6's line `P' = (1 − f) × P + S`, and how
/// many pixels of the fixture are partially covered.
///
/// Its own function because the test that uses it reads three rasters out of the device
/// and would otherwise be one long block of indexing: `before` is the group's content
/// under the element, `shape` carries `f` in its alpha, `deposit` is the element drawn
/// onto transparency, and `actual` is the frame under test.
fn deviation_from_the_clause(
    before: &[u8],
    shape: &[u8],
    deposit: &[u8],
    actual: &[u8],
) -> (f32, u32) {
    let premul = |raster: &[u8], at: usize, channel: usize| {
        f32::from(raster[at + channel]) * f32::from(raster[at + 3]) / 255.0
    };
    let (mut worst, mut partial) = (0.0_f32, 0_u32);
    for pixel in 0..(SIZE * SIZE) as usize {
        let at = pixel * 4;
        let f = f32::from(shape[at + 3]) / 255.0;
        if f > 0.0 && f < 1.0 {
            partial += 1;
        }
        for channel in 0..3 {
            let expected =
                (1.0 - f).mul_add(premul(before, at, channel), premul(deposit, at, channel));
            worst = worst.max((premul(actual, at, channel) - expected).abs());
        }
    }
    (worst, partial)
}

/// **The pair inside a knockout group is the clause's line for that element** (ADR 0032).
///
/// ISO 32000-2 §11.4.6 composites each element with the group's initial backdrop and then
/// takes a weighted average with the immediate backdrop, *weighted by the element's own
/// source shape*:
///
/// > 𝛼gi = (1 − 𝑓si) × 𝛼gi−1 + 𝑓si × 𝛼t
///
/// So the group's per-element rule and the staged pair are the same formula, and an
/// element that states the pair states its own `𝑓si` instead of having it read off the
/// alpha it is drawn with. That is the whole of why the pair belongs here — a nested
/// group or a soft-masked element has a shape the coverage does not carry — and it is
/// why the pair *replaces* the group's erase for that element rather than doubling it.
///
/// The group's first element is opaque and covers the target, so the group buffer is
/// opaque everywhere and compositing it over the page is a copy: what the probe reads is
/// the group's own arithmetic rather than the composite's.
#[test]
fn the_pair_inside_a_knockout_group_is_the_clause() {
    let mut device = device();
    let outline = wedge(&mut device);
    let under = Color::new(0.9, 0.2, 0.1, 1.0);
    // Half-opaque, so shape and opacity are different numbers.
    let object = Color::new(0.1, 0.4, 0.9, 0.5);

    let knockout = GroupSpec {
        alpha: 1.0,
        blend: BlendMode::Normal,
        clip: None,
        knockout: true,
        mask: None,
        isolated: true,
        compose: Compose::SrcOver,
    };
    let cover = |builder: &mut SceneBuilder| {
        builder
            .rect(
                quorra_scene::Rect::new(Point::new(0.0, 0.0), Point::new(SIZE as f32, SIZE as f32)),
                Affine::IDENTITY,
                under,
                None,
                None,
            )
            .unwrap();
    };

    // What the group holds before the staged element: the opaque cover alone.
    let mut before = SceneBuilder::new();
    before
        .group(knockout, |body| {
            cover(body);
            Ok(())
        })
        .unwrap();
    let before = render(&mut device, &before.finish());

    // The two quantities the clause's line is written in, read from the device.
    let onto_transparency = |device: &mut Device, colour: Color, compose: Compose| {
        let mut builder = SceneBuilder::new();
        fill(&mut builder, outline, colour, compose).unwrap();
        render(device, &builder.finish())
    };
    let shape = onto_transparency(
        &mut device,
        Color::new(1.0, 1.0, 1.0, 1.0),
        Compose::SrcOver,
    );
    let deposit = onto_transparency(&mut device, object, Compose::SrcOver);

    let mut staged = SceneBuilder::new();
    staged
        .group(knockout, |body| {
            cover(body);
            fill(
                body,
                outline,
                Color::new(0.0, 0.0, 0.0, 1.0),
                Compose::DestOut,
            )?;
            fill(body, outline, object, Compose::Plus)
        })
        .unwrap();
    let staged = render(&mut device, &staged.finish());

    // And the same element written the way a caller has to write it without the pair:
    // one mark, which reads its shape off the alpha it is drawn with.
    let mut plain = SceneBuilder::new();
    let mask = half(&mut plain).unwrap();
    plain
        .group(knockout, |body| {
            cover(body);
            masked(body, outline, object, mask, Compose::SrcOver)
        })
        .unwrap();
    let plain = render(&mut device, &plain.finish());

    let (worst_staged, partial_pixels) =
        deviation_from_the_clause(&before, &shape, &deposit, &staged);
    let (worst_plain, _) = deviation_from_the_clause(&before, &shape, &deposit, &plain);

    eprintln!("knockout: worst staged {worst_staged:.2}, worst single mark {worst_plain:.2}");
    assert!(
        partial_pixels > 30,
        "the fixture must have partially covered pixels: {partial_pixels}"
    );
    assert!(
        worst_staged <= 3.0,
        "the pair must be §11.4.6's line inside a knockout group too; worst premultiplied \
         deviation {worst_staged}"
    );
    assert!(
        worst_plain >= 16.0,
        "and the single mark must not be — it is the defect the pair exists to fix, and a \
         fixture where the two agree tests nothing: {worst_plain}"
    );
}

/// **A group can be the source of one stage**, which is what §11.6.4.2 forces for a
/// knockout element that is itself a group (ADR 0033).
///
/// > The shape of a group object shall be the union […] of the shapes of the objects it
/// > contains.
///
/// No fill can state that union, so the pair on a *fill* cannot express such an element —
/// which is three of the four pages the caller cannot draw. Written as two groups: the
/// same content opaque under [`Compose::DestOut`], then the content itself under
/// [`Compose::Plus`].
///
/// The element here is two overlapping wedges at half alpha inside their group. Their
/// union is the shape; their alpha is not, and where they overlap the two differ most —
/// which is why the fixture overlaps them.
#[test]
fn a_group_can_be_one_stage_of_the_clause() {
    let mut device = device();
    let outline = wedge(&mut device);
    let shifted = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(24.0, 8.0)),
            Segment::LineTo(Point::new(56.0, 40.0)),
            Segment::LineTo(Point::new(24.0, 56.0)),
            Segment::Close,
        ])
        .unwrap();
    let object = Color::new(0.1, 0.4, 0.9, 0.5);

    let stage = |compose: Compose| GroupSpec {
        alpha: 1.0,
        blend: BlendMode::Normal,
        clip: None,
        knockout: false,
        mask: None,
        isolated: true,
        compose,
    };
    let content = |body: &mut SceneBuilder, colour: Color| -> Result<(), SceneError> {
        fill(body, outline, colour, Compose::SrcOver)?;
        fill(body, shifted, colour, Compose::SrcOver)
    };
    let opaque = Color::new(1.0, 1.0, 1.0, 1.0);

    // f and S, read from the device: the shape group and the object group on their own.
    let onto_transparency = |device: &mut Device, colour: Color| {
        let mut builder = SceneBuilder::new();
        builder
            .group(stage(Compose::SrcOver), |body| content(body, colour))
            .unwrap();
        render(device, &builder.finish())
    };
    let shape = onto_transparency(&mut device, opaque);
    let deposit = onto_transparency(&mut device, object);

    let mut before = SceneBuilder::new();
    backdrop(&mut before);
    let before = render(&mut device, &before.finish());

    let mut staged = SceneBuilder::new();
    backdrop(&mut staged);
    staged
        .group(stage(Compose::DestOut), |body| content(body, opaque))
        .unwrap();
    staged
        .group(stage(Compose::Plus), |body| content(body, object))
        .unwrap();
    let staged = render(&mut device, &staged.finish());

    let mut plain = SceneBuilder::new();
    backdrop(&mut plain);
    plain
        .group(stage(Compose::SrcOver), |body| content(body, object))
        .unwrap();
    let plain = render(&mut device, &plain.finish());

    let (worst_staged, partial) = deviation_from_the_clause(&before, &shape, &deposit, &staged);
    let (worst_plain, _) = deviation_from_the_clause(&before, &shape, &deposit, &plain);
    eprintln!("group stage: worst staged {worst_staged:.2}, worst ordinary {worst_plain:.2}");
    assert!(
        partial > 30,
        "the fixture needs partial coverage: {partial}"
    );
    assert!(
        worst_staged <= 3.0,
        "a group as one stage must be §11.4.6's line; worst deviation {worst_staged}"
    );
    assert!(
        worst_plain >= 16.0,
        "and the ordinary composite must not be, or the fixture proves nothing: \
         {worst_plain}"
    );
}

/// The two positions a group may not be composited from, and why each is refused.
#[test]
fn a_group_refuses_the_stages_where_the_clause_does_not_put_them() {
    let spec = |compose: Compose, blend: BlendMode, isolated: bool| GroupSpec {
        alpha: 1.0,
        blend,
        clip: None,
        knockout: false,
        mask: None,
        isolated,
        compose,
    };
    let refuse = |compose, blend, isolated| {
        SceneBuilder::new()
            .group(spec(compose, blend, isolated), |_| Ok(()))
            .expect_err("refused")
    };
    assert_eq!(
        refuse(Compose::DestOut, BlendMode::Multiply, true),
        SceneError::GroupComposeUnsupported {
            compose: Compose::DestOut,
            reason: GroupComposeReason::BlendNotNormal,
        }
    );
    assert_eq!(
        refuse(Compose::Plus, BlendMode::Normal, false),
        SceneError::GroupComposeUnsupported {
            compose: Compose::Plus,
            reason: GroupComposeReason::NonIsolated,
        }
    );
    assert_eq!(
        refuse(Compose::Src, BlendMode::Normal, true),
        SceneError::GroupComposeUnsupported {
            compose: Compose::Src,
            reason: GroupComposeReason::Source,
        },
        "§11.4.6 calls that a knockout group, and `GroupSpec::knockout` states it"
    );
    SceneBuilder::new()
        .group(
            spec(Compose::SrcOver, BlendMode::Multiply, true),
            |_| Ok(()),
        )
        .expect("an ordinary group is untouched by ADR 0033");
}

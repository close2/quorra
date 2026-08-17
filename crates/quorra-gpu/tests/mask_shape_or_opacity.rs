//! A soft mask is a knockout element's **opacity**, not its shape (ADR 0066).
//!
//! # Where the expected values come from
//!
//! ISO 32000-2 §11.6.4.3 does not decide this on its own:
//!
//! > The mask may serve as a source of either shape ( fm ) or opacity ( qm ) values,
//! > depending on the setting of the alpha source parameter in the graphics state
//!
//! Table 57 decides it, and it decides the constant alpha in the same sentence:
//!
//! > alpha source … A flag specifying whether the current soft mask and alpha constant
//! > parameters shall be interpreted as shape values ( true ) or opacity values
//! > ( false ). … Initial value: false .
//!
//! §11.6.4.4 says the same from the other end — "the AIS ('alpha is shape') entry in a
//! graphics state parameter dictionary shall determine whether the alpha constants are
//! interpreted as shape values ( true ) or opacity values ( false )". One flag, two
//! parameters, and **a `Scene` carries no such flag**, so both take the initial value:
//! the mask is `qm` and the paint's alpha is `qk`.
//!
//! The difference is invisible wherever only the product `f × q` is used, which is
//! everywhere §11.3.6 composites. §11.4.6 is the one place that reads them apart:
//!
//! > The existence of the knockout feature is the main reason for maintaining a separate
//! > shape value rather than only a single alpha that combines shape and opacity.
//!
//! So the expectation for a masked element of an isolated knockout group is the same
//! line every other file here measures — `P' = (1 − f) × P + S`, per channel and
//! premultiplied (`common::clause`) — with `f` the element's **geometry alone**
//! (§11.6.4.2, met with §8.5.4's clip) and the mask living entirely inside `S`.
//!
//! # What makes this fixture able to see the question
//!
//! The other reading — the mask as `fm`, which is what every lane's `fs_shape` computed
//! until 2026-08-18 — is the same line with `f` replaced by `coverage × mask`. Each test
//! measures the frame against **both**, and requires the second to miss: a fixture whose
//! mask is 1, or 0, or whose element has no partially covered pixel, would satisfy the
//! first assertion while proving nothing. [`the_mask_this_file_draws_is_not_trivial`]
//! holds the fixture itself to that, read from the device rather than assumed.

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

use std::collections::BTreeSet;
use std::sync::Arc;

use quorra_gpu::{Device, Options};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, GroupSpec, ImageFilter, ImageId, ImageSpec,
    MaskId, MaskKind, OutlineId, Paint, Point, Rect, SceneBuilder, SceneError, Segment,
};

mod common;

use common::clause::deviation_from_the_clause;
use common::headless::render;

const SIZE: u32 = 64;

/// The device this file's assertions are made on, with `QUORRA_ADAPTER` able to name
/// another — `function_lane.rs`'s idiom rather than `common::headless`'s pinned software
/// rasteriser.
///
/// Every assertion here is a **difference between frames drawn on one adapter**: a bound
/// on the deviation from §11.4.6's line, a bound that the other reading misses it, and
/// one byte equality between two frames of the same device. None of them is a claim about
/// a byte across adapters, which is what pins the rest of the suite, so this file can be
/// pointed at the real GPU and answer the same clause question there.
fn device() -> Device {
    let requested = std::env::var("QUORRA_ADAPTER").unwrap_or_else(|_| "llvmpipe".into());
    Device::headless(&Options {
        adapter: Some(requested),
        ..Options::default()
    })
    .expect("the requested adapter is present")
}

/// The opaque content the element lands on inside the group: full-target, so the group's
/// buffer is opaque under the element and compositing the finished group over the empty
/// page is a copy — what the probe reads is the group's own arithmetic.
const UNDER: Color = Color {
    r: 0.9,
    g: 0.2,
    b: 0.1,
    a: 1.0,
};

/// The element's paint. Half-opaque, so its shape and its opacity are different numbers
/// before the mask is even considered.
const OBJECT: Color = Color {
    r: 0.1,
    g: 0.4,
    b: 0.9,
    a: 0.5,
};

/// A triangle with a diagonal edge, so partially covered pixels exist: a fixture of
/// axis-aligned rectangles would agree while being wrong (§4.1 of the brief).
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

/// A 2 × 2 opaque white image: the image lane's element, whose own alpha is 1 so that
/// the mask is the only opacity the test varies.
fn white_image(device: &mut Device) -> ImageId {
    device
        .upload_image(&ImageSpec {
            width: 2,
            height: 2,
            data: Arc::from(vec![255_u8; 16].into_boxed_slice()),
        })
        .unwrap()
}

/// §8.9.5.1 places every image on the unit square, so a placement is a rectangle written
/// as an affine. Fractional corners on purpose: an image rectangle on integer boundaries
/// has no partially covered pixel, and `f` would then be 0 or 1 everywhere.
fn placement() -> Affine {
    Affine {
        a: 43.4,
        b: 0.0,
        c: 0.0,
        d: 43.4,
        e: 10.3,
        f: 10.3,
    }
}

/// §11.5.2's alpha mask, in two bands.
///
/// Two overlapping half-transparent rectangles reduce to a mask that is 0.4 on the right,
/// 0.7 on the left and a fraction of the way between at the column the second rectangle's
/// edge crosses — three values, none of them 0 or 1, and varying *across* the element
/// rather than only under part of it. A uniform mask would answer the same question, but
/// a mask that varies is the one whose product with a coverage cannot be mistaken for a
/// coverage.
fn banded_mask(builder: &mut SceneBuilder) -> Result<MaskId, SceneError> {
    builder.mask(MaskKind::Alpha, None, |body| {
        body.rect(
            Rect::new(Point::new(0.0, 0.0), Point::new(SIZE as f32, SIZE as f32)),
            Affine::IDENTITY,
            Color::new(1.0, 1.0, 1.0, 0.4),
            None,
            None,
        )?;
        body.rect(
            Rect::new(Point::new(0.0, 0.0), Point::new(31.5, SIZE as f32)),
            Affine::IDENTITY,
            Color::new(1.0, 1.0, 1.0, 0.5),
            None,
            None,
        )
    })
}

/// The isolated knockout group this file measures inside.
fn knockout() -> GroupSpec {
    GroupSpec {
        alpha: 1.0,
        blend: BlendMode::Normal,
        clip: None,
        knockout: true,
        mask: None,
        isolated: true,
        compose: Compose::SrcOver,
    }
}

/// The opaque cover the group holds before the element.
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

/// One element of a group, at a stated colour, with or without the mask — the two lanes
/// this file draws differ only in this function.
type Element = fn(&mut SceneBuilder, &mut Device, Color, Option<MaskId>, Compose);

fn filled(
    builder: &mut SceneBuilder,
    device: &mut Device,
    colour: Color,
    mask: Option<MaskId>,
    compose: Compose,
) {
    let outline = wedge(device);
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Solid(colour),
            None,
            BlendMode::Normal,
            compose,
            mask,
        )
        .unwrap();
}

/// The image lane's element. Its colour argument is used for one thing only — the alpha,
/// which is §11.6.4.4's constant — because an image carries its own samples.
fn imaged(
    builder: &mut SceneBuilder,
    device: &mut Device,
    colour: Color,
    mask: Option<MaskId>,
    _compose: Compose,
) {
    let image = white_image(device);
    builder
        .image(
            image,
            placement(),
            colour.a,
            ImageFilter::Nearest,
            None,
            BlendMode::Normal,
            mask,
        )
        .unwrap();
}

/// The four rasters §11.4.6's line is written in, all read from the device.
struct Quantities {
    /// `P`: the group holding everything before the element.
    before: Vec<u8>,
    /// `f` under §11.6.4.3 with Table 57's initial alpha source: geometry alone.
    shape: Vec<u8>,
    /// `f` under the other reading: geometry times the mask.
    shape_if_the_mask_were_shape: Vec<u8>,
    /// `S`: the element's own premultiplied deposit, mask and constant alpha included.
    deposit: Vec<u8>,
    /// The frame under test: the element inside the knockout group.
    actual: Vec<u8>,
}

fn measure(device: &mut Device, element: Element) -> Quantities {
    let mut before = SceneBuilder::new();
    before
        .group(knockout(), |body| {
            cover(body);
            Ok(())
        })
        .unwrap();
    let before = render(device, &before.finish(), SIZE, SIZE);

    // Onto transparency, at the root: §11.3.6 against `ab = 0` leaves `co = as·Cs` and
    // `ao = as`, so the alpha of such a raster *is* the source alpha the mark carries.
    let onto_transparency = |device: &mut Device, colour: Color, masked: bool| {
        let mut builder = SceneBuilder::new();
        let mask = masked.then(|| banded_mask(&mut builder).unwrap());
        element(&mut builder, device, colour, mask, Compose::SrcOver);
        render(device, &builder.finish(), SIZE, SIZE)
    };
    let opaque = Color::new(1.0, 1.0, 1.0, 1.0);

    // A mask is defined before the command that references it (`SceneBuilder::mask`
    // returns an id that is valid only afterwards, which is what keeps mask
    // dependencies acyclic), so it is allocated on this scene before the group opens.
    let mut actual = SceneBuilder::new();
    let mask = banded_mask(&mut actual).unwrap();
    actual
        .group(knockout(), |body| {
            cover(body);
            element(body, device, OBJECT, Some(mask), Compose::SrcOver);
            Ok(())
        })
        .unwrap();
    let actual = render(device, &actual.finish(), SIZE, SIZE);

    Quantities {
        before,
        shape: onto_transparency(device, opaque, false),
        shape_if_the_mask_were_shape: onto_transparency(device, opaque, true),
        deposit: onto_transparency(device, OBJECT, true),
        actual,
    }
}

/// The two assertions every lane owes the clause, so a lane test is one call.
fn assert_the_mask_is_opacity(lane: &str, q: &Quantities) {
    let (worst_opacity, partial) =
        deviation_from_the_clause(&q.before, &q.shape, &q.deposit, &q.actual);
    let (worst_shape, _) = deviation_from_the_clause(
        &q.before,
        &q.shape_if_the_mask_were_shape,
        &q.deposit,
        &q.actual,
    );
    eprintln!("{lane}: worst as opacity {worst_opacity:.2}, worst as shape {worst_shape:.2}");
    assert!(
        partial > 30,
        "the fixture must have partially covered pixels for this to mean anything: {partial}"
    );
    assert!(
        worst_opacity <= 3.0,
        "Table 57's alpha source flag is `false` by default and no scene can set it, so a \
         masked {lane}'s shape is its geometry and §11.4.6 erases by that alone. Worst \
         premultiplied deviation {worst_opacity}"
    );
    assert!(
        worst_shape >= 16.0,
        "and the mask-as-shape reading must miss the same line — a fixture where the two \
         readings agree holds nothing: {worst_shape}"
    );
}

/// **The defect this file was written for**, on the coverage lane: `fs_shape` multiplied
/// the mark's soft mask into the shape it returned, which is §11.6.4.3's `AIS = true`
/// reading applied unconditionally.
#[test]
fn a_masked_fill_in_a_knockout_group_is_erased_by_its_geometry_alone() {
    let mut device = device();
    let measured = measure(&mut device, filled);
    assert_the_mask_is_opacity("fill", &measured);
}

/// The same clause on the image lane, where the element has three sources of opacity —
/// its own samples, §11.6.4.4's constant and the mask — and one source of shape, which is
/// §11.6.4.2's image rectangle.
#[test]
fn a_masked_image_in_a_knockout_group_is_erased_by_its_rectangle() {
    let mut device = device();
    let measured = measure(&mut device, imaged);
    assert_the_mask_is_opacity("image", &measured);
}

/// ADR 0025's staged erase reads the element's shape, so the mark's mask may not change
/// what it erases — the same statement `staged_compose.rs` makes about the paint's alpha,
/// about the other parameter Table 57's one flag governs.
///
/// Byte equality rather than a bound: the two scenes differ in one argument, and if the
/// mask is not in the shape they are the same frame exactly.
#[test]
fn the_staged_erase_does_not_read_the_marks_mask() {
    let mut device = device();
    let erase = |device: &mut Device, masked: bool, compose: Compose| {
        let mut builder = SceneBuilder::new();
        let mask = masked.then(|| banded_mask(&mut builder).unwrap());
        cover(&mut builder);
        filled(&mut builder, device, OBJECT, mask, compose);
        render(device, &builder.finish(), SIZE, SIZE)
    };

    assert_eq!(
        erase(&mut device, true, Compose::DestOut),
        erase(&mut device, false, Compose::DestOut),
        "§11.6.4.3's mask is `qm` under Table 57's initial alpha source, and \
         `Compose::DestOut` weights by shape"
    );

    // The control: the same two marks composited the ordinary way must differ, or the
    // equality above would be a statement about a mask that never reached the mark.
    assert_ne!(
        erase(&mut device, true, Compose::SrcOver),
        erase(&mut device, false, Compose::SrcOver),
        "the mask must be reaching this mark at all, or the assertion above is vacuous"
    );
}

/// **And the other reading is not lost, it is spelled.** A caller holding a page painted
/// under `/AIS true` — nine documents of their corpus state the entry — can say
/// `coverage × mask` as a shape with ADR 0033's group stages, because the erase weight of
/// a `Compose::DestOut` group is that group's own alpha times its soft mask, and a group's
/// alpha is the only way a group's shape reaches a raster at all.
///
/// So this library refuses nothing here and approximates nothing: the default reading is
/// what an unstated scene draws, and the flagged reading is a construction with a name.
/// Held to §11.4.6's line with `f = coverage × mask`, and required to miss the line with
/// `f = coverage`, which is the same pair of assertions the other tests make in reverse.
#[test]
fn the_shape_reading_is_expressible_with_the_group_stages() {
    let mut device = device();
    let q = measure(&mut device, filled);

    let stage = |compose: Compose, mask: MaskId| GroupSpec {
        alpha: 1.0,
        blend: BlendMode::Normal,
        clip: None,
        knockout: false,
        mask: Some(mask),
        isolated: true,
        compose,
    };
    // Uploaded once, so the closures below need the builder alone.
    let outline = wedge(&mut device);
    let unmasked = |body: &mut SceneBuilder, colour: Color| {
        body.fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Solid(colour),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
    };
    let mut staged = SceneBuilder::new();
    let mask = banded_mask(&mut staged).unwrap();
    staged
        .group(knockout(), |body| {
            cover(body);
            // The shape half: the element drawn opaque and *unmasked* inside a group the
            // mask is applied to, so the erase weighs `coverage × mask`.
            body.group(stage(Compose::DestOut, mask), |inner| {
                unmasked(inner, Color::new(1.0, 1.0, 1.0, 1.0))
            })?;
            body.group(stage(Compose::Plus, mask), |inner| unmasked(inner, OBJECT))
        })
        .unwrap();
    let staged = render(&mut device, &staged.finish(), SIZE, SIZE);

    let (worst_shape, partial) = deviation_from_the_clause(
        &q.before,
        &q.shape_if_the_mask_were_shape,
        &q.deposit,
        &staged,
    );
    let (worst_opacity, _) = deviation_from_the_clause(&q.before, &q.shape, &q.deposit, &staged);
    eprintln!("staged: worst as shape {worst_shape:.2}, worst as opacity {worst_opacity:.2}");
    assert!(
        partial > 30,
        "the fixture must have partially covered pixels: {partial}"
    );
    assert!(
        worst_shape <= 3.0,
        "ADR 0033's stages let a caller state `coverage × mask` as the shape, which is \
         §11.6.4.3 under `/AIS true`; worst premultiplied deviation {worst_shape}"
    );
    assert!(
        worst_opacity >= 16.0,
        "and it must be the *other* line from the one the default draws, or this \
         construction says nothing: {worst_opacity}"
    );
}

/// **The fixture held to its own claim.** Over the pixels the element covers fully, the
/// mask must take at least three distinct values and none of them may be 0 or 1 —
/// otherwise "the two readings disagree" would be a property of this file's arithmetic
/// rather than of the frames it draws.
///
/// The count is of *distinct values*, not of masked pixels: a mask that is one number
/// everywhere is still a mask, and a hit rate would call it non-trivial.
#[test]
fn the_mask_this_file_draws_is_not_trivial() {
    let mut device = device();
    let q = measure(&mut device, filled);
    let mut values = BTreeSet::new();
    for at in (0..q.shape.len()).step_by(4) {
        if q.shape[at + 3] == 255 {
            values.insert(q.shape_if_the_mask_were_shape[at + 3]);
        }
    }
    eprintln!("mask values under full coverage: {values:?}");
    assert!(
        values.len() >= 3,
        "the mask must vary across the element, or one band could be a coverage: {values:?}"
    );
    assert!(
        values.iter().all(|value| *value > 0 && *value < 255),
        "and none of its values may be 0 or 1, where shape and opacity coincide: {values:?}"
    );
}

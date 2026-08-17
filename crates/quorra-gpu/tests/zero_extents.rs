//! A zero-sized thing is a legal thing for a document to ask for, and each kind of it has
//! its own answer.
//!
//! `doc/notes-ceilings-audit.md` §4 is the round this file witnesses, and the question is
//! the caller's, from `pdf-viewer/doc/HAYRO_ISSUES_FOR_QUORRA.md` §1 on their `#351`,
//! `#352` and `#357`:
//!
//! > A zero-width surface is a legal thing for a document to ask for, and the question
//! > every renderer answers eventually is whether the zero is caught at the top (where a
//! > frame can be skipped) or at the bottom (where a `Vec` is indexed).
//!
//! `doc/PLAN.md` already states the intended answer — "a blank scene is a legitimate
//! scene, and so is a zero-length buffer slice that follows from one" — so what is wanted
//! here is whether it holds for every kind, gated rather than asserted in prose. **Two
//! kinds of answer, and each test says which it is:**
//!
//! - **`Ok` with nothing drawn** wherever the zero is a picture: a zero-size readback, a
//!   layer whose plan marks nothing, a soft mask whose group marks nothing, a coverage
//!   tile that rounds to no pixel. Each of these is a *legitimate frame*, and refusing it
//!   would be the "cache filled up" mistake `encode/residue.rs` names — a frame that could
//!   be drawn and was not.
//! - **A named `Err`** wherever the zero is an impossibility rather than a picture: a
//!   `Surface` and a `Texture` cannot exist at zero size, so a zero-size viewport aimed at
//!   one is [`RenderError::ZeroSizeTarget`] naming which.
//!
//! **Where the other kinds are gated**, since a fixture with two copies is two fixtures:
//! `m1.rs`'s `zero_size_targets_are_handled_by_kind` crosses the zero *viewport* into a
//! `Readback` (`Ok`, an empty raster) and into a `Texture` (`Err`); `atlas.rs`'s
//! `a_tile_with_a_zero_side_is_never_admitted` and `encode/scratch.rs`'s
//! `a_tile_with_a_zero_side_takes_no_place_on_the_sheet` are the two packers' own doors,
//! stated where the shelf arithmetic is.

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

use quorra_gpu::{Device, Options, RenderError, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, GroupSpec, MaskKind, Paint, Point, Scene,
    SceneBuilder, Segment,
};

const SIZE: u32 = 32;

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
        .expect("the frame draws")
        .into_raster()
        .unwrap()
        .into_pixels()
}

fn is_blank(pixels: &[u8]) -> bool {
    pixels.iter().skip(3).step_by(4).all(|&a| a == 0)
}

fn black() -> Paint {
    Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0))
}

fn plain_group() -> GroupSpec {
    GroupSpec {
        alpha: 1.0,
        blend: BlendMode::Normal,
        clip: None,
        knockout: false,
        mask: None,
        isolated: true,
        compose: Compose::SrcOver,
    }
}

/// **A zero-size viewport aimed at a `Surface` is a named refusal** — the half of
/// `RenderError::ZeroSizeTarget` `m1.rs` does not reach, because it has no window.
///
/// The check is above the target binding on purpose, which is what this asserts: a
/// headless device has no surface at all, and the answer is still the zero, not
/// `NoSurface`. A caller who resized a window to nothing needs to hear which of the two
/// it was, because one is a frame to skip and the other is a device to rebuild.
#[test]
fn a_zero_size_surface_target_is_refused_by_name() {
    let mut device = device();
    let scene = SceneBuilder::new().finish();
    match device.render(
        &scene,
        &Viewport::full(0, SIZE, Affine::IDENTITY),
        Target::Surface,
    ) {
        Err(RenderError::ZeroSizeTarget { target: "Surface" }) => {}
        other => panic!("expected ZeroSizeTarget for a Surface, got {other:?}"),
    }
    match device.render(
        &scene,
        &Viewport::full(SIZE, 0, Affine::IDENTITY),
        Target::Surface,
    ) {
        Err(RenderError::ZeroSizeTarget { target: "Surface" }) => {}
        other => panic!("expected ZeroSizeTarget for a Surface, got {other:?}"),
    }
}

/// **A layer whose plan marks nothing is drawn, not refused** — `Ok`, a blank page, and a
/// texture that exists.
///
/// `compose::Region::of` gives a plan with no bounds **one texel rather than none**,
/// because wgpu refuses a zero-sized texture and a composite still reads whatever the plan
/// left — a cleared texel, which contributes nothing.
///
/// The counters say which shape this fixture actually is, so that a later change cannot
/// leave it passing while exercising something else: the group is a real layer by §11.4.5
/// (it has a constant alpha), its body marks nothing, so ADR 0041 **culls the child**
/// (`layers_culled == 1`) and what is left holding no bounds is the frame's root
/// accumulator (`layer_textures == 1`). That accumulator is the `bounds: None` case,
/// reached through the public API rather than by calling `Region::of` directly.
#[test]
fn a_layer_whose_plan_marks_nothing_draws_a_blank_frame() {
    let mut device = device();
    let mut builder = SceneBuilder::new();
    builder
        .group(
            GroupSpec {
                alpha: 0.5,
                ..plain_group()
            },
            |_| Ok(()),
        )
        .unwrap();
    let frame = device
        .render(
            &builder.finish(),
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("a layer that marks nothing is a frame, not a refusal");
    let counters = frame.counters();
    assert_eq!(
        counters.layers_culled, 1,
        "the empty child contributes nothing"
    );
    assert_eq!(
        counters.layer_textures, 1,
        "and the root accumulator has no bounds"
    );
    let pixels = frame.into_raster().unwrap().into_pixels();
    assert!(is_blank(&pixels), "an empty group marks no pixel");
}

/// **A soft mask whose group marks nothing masks everything**, and the frame is `Ok`.
///
/// ISO 32000-2 §11.6.5.2 derives an alpha mask from the alpha of the mask group's result,
/// and a group that marks nothing has alpha zero everywhere — so the fill it masks
/// contributes nothing. That is a picture, not a failure, and it is exactly
/// `CoverageMask::transparent`'s documented meaning: "an empty clip region admits nothing
/// *inside* it too, which is a different statement from having no clip".
#[test]
fn a_soft_mask_whose_group_marks_nothing_masks_everything() {
    let mut device = device();
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(4.0, 4.0)),
            Segment::LineTo(Point::new(28.0, 6.0)),
            Segment::LineTo(Point::new(28.0, 28.0)),
            Segment::LineTo(Point::new(4.0, 26.0)),
            Segment::Close,
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    let mask = builder.mask(MaskKind::Alpha, None, |_| Ok(())).unwrap();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            black(),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            Some(mask),
        )
        .unwrap();
    let pixels = render(&mut device, &builder.finish());
    assert!(
        is_blank(&pixels),
        "a mark under an empty alpha mask deposits nothing"
    );
}

/// **A mark whose device box rounds to no pixel in one axis is drawn as nothing**, `Ok`,
/// with the rest of the page untouched.
///
/// This is the caller's `#351`/`#352`/`#357` shape one level down from the page: their
/// `/MediaBox` rounds to zero pixels in one axis, ours is a path whose device extent does.
/// `coverage_tile` returns `None` before a tile is charged or allocated, which is the
/// "zero-length buffer slice that follows from a blank scene" `doc/PLAN.md` calls
/// legitimate — and the second mark is here so that a frame which gave up on the *page*
/// rather than on the *mark* would be visible.
#[test]
fn a_mark_with_a_zero_extent_axis_draws_nothing_and_stops_no_frame() {
    let mut device = device();
    let flat = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(4.0, 10.0)),
            Segment::LineTo(Point::new(28.0, 10.0)),
            Segment::Close,
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            flat,
            Affine::IDENTITY,
            FillRule::NonZero,
            black(),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    builder
        .rect(
            quorra_scene::Rect::new(Point::new(20.0, 20.0), Point::new(28.0, 28.0)),
            Affine::IDENTITY,
            Color::new(0.0, 0.0, 0.0, 1.0),
            None,
            None,
        )
        .unwrap();
    let pixels = render(&mut device, &builder.finish());
    let at = |x: u32, y: u32| pixels[((y * SIZE + x) * 4 + 3) as usize];
    assert_eq!(at(16, 10), 0, "the flat path inks no pixel of its own row");
    assert_eq!(at(16, 9), 0, "nor the row above it");
    assert_eq!(at(24, 24), 255, "and the frame kept drawing after it");
}

//! A mark that contributes nothing leaves the target **byte-identical**: ISO 32000-2
//! §11.4.6, §11.6.4.2.
//!
//! The question comes from the caller's reading of hayro #4, the `/None` colourant.
//! §8.6.6.4 is unusually blunt about it:
//!
//! > The special colourant name None shall not produce any visible output. Painting
//! > operations in a Separation space with this colourant name shall have no effect on the
//! > current page.
//!
//! `/None` itself never reaches this library — colour is not ours (`PLAN.md` integration
//! note 6: `ColourSpace::to_rgb` upstream is the only place a colour becomes RGB), so a
//! `/None` painting operation is one the caller does not issue. What *is* ours is the
//! property that sentence is the cheapest test of: **a mark that contributes nothing must
//! be a no-op in the compositor, not an operation that happens to compute the same
//! answer.** A compositor that reaches the right pixel by a wrong route is one rounding
//! step away from a wrong page, and §11.4.6 is where that step lives, because a knockout
//! group *replaces* where an ordinary one composites.
//!
//! # The two halves of "nothing", which the clause does not treat alike
//!
//! §11.6.4.2 separates an object's **shape** from its **opacity**, and says the separation
//! exists for exactly this reason (§11.4.6): "The existence of the knockout feature is the
//! main reason for maintaining a separate shape value rather than only a single alpha that
//! combines shape and opacity."
//!
//! - **No shape.** An empty clip, a zero-area rectangle, a zero-area outline: the element's
//!   source shape is 0. §11.4.6's NOTE 5 is unconditional — "A shape value of 0.0 (outside)
//!   leaves the previous group results unchanged" — so this is a no-op in every kind of
//!   group, and every lane and every group kind is held to byte identity.
//! - **No opacity.** A fill whose paint alpha is 0 has, in the clause's terms, shape 1
//!   inside its path (§11.6.4.2: "the shape shall always be 1.0 inside and 0.0 outside the
//!   path") and constant opacity 0 (§11.6.4.4). Outside a knockout group that composites to
//!   the backdrop unchanged. **Inside one it does not**: §11.4.6 composites the element with
//!   the group's *initial* backdrop and then replaces a shape-fraction of the accumulated
//!   group with the result, and a transparent element over a transparent initial backdrop
//!   is transparent. So a zero-opacity mark inside a knockout group erases what it covers,
//!   and a compositor that made it a no-op would be wrong. Both halves are asserted below.
//!
//! **A fully transparent soft mask is in the second half, not the first** (ADR 0066). It
//! reads as shape only under Table 57's alpha source flag, whose initial value is `false`
//! and which a scene cannot set: "A flag specifying whether the current soft mask and alpha
//! constant parameters shall be interpreted as shape values ( true ) or opacity values
//! ( false ). … Initial value: false ." So a mask of 0 is `qm = 0` over a shape of 1, and
//! inside a knockout group it erases exactly as a zero paint alpha does. This file asserted
//! the opposite until 2026-08-18, and the shader agreed with it.
//!
//! **What this means for `/None`, and it is worth the caller having in writing**: `/None`
//! cannot be modelled as a zero-alpha paint. Inside a knockout group the two differ — one
//! is required to change nothing and the other is required to erase — so a painting
//! operation in a `/None` Separation space has to be a command that is never issued, which
//! is what `pdf-model` already does by deciding the colourant before the tint transform.
//!
//! # One question, at 600 lines
//!
//! `CLAUDE.md`'s ~500-line smell asks a file past it to say what its one irreducible thing
//! is. This file's one thing is the sentence at the top, and its length is the *matrix*
//! that sentence needs rather than a second responsibility: six ways for a mark to
//! contribute nothing × four things it can be nested in, with one page fixture shared by
//! all of them and one control that proves the fixture discriminates. Splitting it along
//! any seam here — by kind, or by group — would compare a lane with itself in one half and
//! leave the control in the other, which is the defect ADR 0047 found in `m45.rs`.

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
use quorra_gpu::Device;
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, GroupSpec, ImageFilter, ImageSpec, MaskKind,
    Paint, Point, Rect, SceneBuilder, SceneError, Segment,
};
use std::sync::Arc;

/// 64 pixels wide: 64 × 4 bytes = 256, the buffer-copy row alignment.
const SIZE: u32 = 64;

/// The rectangle every "nothing" below is asked to cover. Large, and overlapping every
/// part of the backdrop, so that a mark which did contribute would be seen on opaque
/// pixels, on half-transparent ones and on the antialiased edge between them.
const OVER: Rect = Rect {
    min: Point::new(8.0, 8.0),
    max: Point::new(56.0, 56.0),
};

/// A way for a mark to contribute nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Nothing {
    /// A clip chain of two disjoint rectangles: §4.7's "an empty clip admits nothing —
    /// which is different from an absent clip", and ADR 0007's empty resolution.
    EmptyClip,
    /// A soft mask whose group marks nothing at all, so the reduction is 0 everywhere
    /// (§11.5.2 with the identity transfer): **shape 1, opacity 0**, because Table 57's
    /// alpha source flag is `false` by default and a scene carries no other value
    /// (§11.6.4.3, ADR 0066).
    TransparentSoftMask,
    /// A degenerate rectangle through the analytic rectangle lane (`rect.wgsl`).
    ZeroAreaRect,
    /// Three collinear points through the coverage lane: a fill enclosing no area
    /// (§8.5.3.3's rules decide a region, and this outline bounds none).
    ZeroAreaFill,
    /// A fill whose paint alpha is 0: **shape 1, opacity 0** (§11.6.4.2, §11.6.4.4).
    ZeroAlphaFill,
    /// An image drawn at constant alpha 0: shape is the image rectangle (§11.6.4.2, "For
    /// images … the shape shall be 1.0 inside the image rectangle"), opacity 0.
    ZeroAlphaImage,
}

/// The kinds whose **shape** is zero. A no-op in every group, by §11.4.6's NOTE 5.
const NO_SHAPE: [Nothing; 3] = [
    Nothing::EmptyClip,
    Nothing::ZeroAreaRect,
    Nothing::ZeroAreaFill,
];

/// The kinds whose **opacity** is zero while their shape is not. A no-op everywhere except
/// inside a knockout group, where §11.4.6 replaces rather than composites.
const NO_OPACITY: [Nothing; 3] = [
    Nothing::ZeroAlphaFill,
    Nothing::ZeroAlphaImage,
    Nothing::TransparentSoftMask,
];

/// What the "nothing" mark is nested in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Context {
    /// Straight onto the page's own group (§11.4.7).
    Root,
    /// §11.4.5's isolated, non-knockout group — a layer.
    Isolated,
    /// §11.4.6's knockout group.
    Knockout,
    /// §11.4.4's non-isolated group, whose layer is seeded with the backdrop.
    NonIsolated,
}

const CONTEXTS: [Context; 4] = [
    Context::Root,
    Context::Isolated,
    Context::Knockout,
    Context::NonIsolated,
];

fn spec(context: Context) -> GroupSpec {
    GroupSpec {
        alpha: 1.0,
        blend: BlendMode::Normal,
        clip: None,
        knockout: context == Context::Knockout,
        mask: None,
        isolated: context != Context::NonIsolated,
        compose: Compose::SrcOver,
    }
}

/// A 2 × 2 opaque white image, the smallest thing the image lane will draw.
fn white_image() -> ImageSpec {
    ImageSpec {
        width: 2,
        height: 2,
        data: Arc::from(vec![255_u8; 16].into_boxed_slice()),
    }
}

fn triangle(a: Point, b: Point, c: Point) -> Vec<Segment> {
    vec![
        Segment::MoveTo(a),
        Segment::LineTo(b),
        Segment::LineTo(c),
        Segment::Close,
    ]
}

/// The page under everything: an opaque square, a half-transparent square overlapping it,
/// and a triangle whose diagonal edge gives the frame a column of fractional coverage.
///
/// Three kinds of backdrop pixel on purpose. Byte identity over an opaque flat region is a
/// weak claim; over a pixel whose alpha is 128 and whose colour is premultiplied, and over
/// one whose coverage is a fraction, it is the claim worth making — those are the pixels a
/// re-composite that "computes the same answer" would round differently.
fn backdrop(builder: &mut SceneBuilder, device: &mut Device) -> Result<(), SceneError> {
    builder.rect(
        Rect::new(Point::new(4.0, 4.0), Point::new(40.0, 40.0)),
        Affine::IDENTITY,
        Color::new(0.9, 0.1, 0.1, 1.0),
        None,
        None,
    )?;
    builder.rect(
        Rect::new(Point::new(24.0, 24.0), Point::new(60.0, 60.0)),
        Affine::IDENTITY,
        Color::new(0.1, 0.2, 0.9, 0.5),
        None,
        None,
    )?;
    let wedge = device
        .upload_outline(&triangle(
            Point::new(6.0, 58.0),
            Point::new(58.0, 58.0),
            Point::new(6.0, 6.0),
        ))
        .expect("upload");
    builder.fill(
        wedge,
        Affine::IDENTITY,
        FillRule::NonZero,
        Paint::Solid(Color::new(0.1, 0.8, 0.3, 0.6)),
        None,
        BlendMode::Normal,
        Compose::SrcOver,
        None,
    )
}

/// The mark that goes *inside* the group, before the nothing: something for a knockout to
/// knock out, and something for a non-isolated group's interpolation to act on.
fn group_content(builder: &mut SceneBuilder, device: &mut Device) -> Result<(), SceneError> {
    let wedge = device
        .upload_outline(&triangle(
            Point::new(12.0, 12.0),
            Point::new(52.0, 12.0),
            Point::new(52.0, 52.0),
        ))
        .expect("upload");
    builder.fill(
        wedge,
        Affine::IDENTITY,
        FillRule::NonZero,
        Paint::Solid(Color::new(0.95, 0.85, 0.1, 1.0)),
        None,
        BlendMode::Normal,
        Compose::SrcOver,
        None,
    )
}

/// Whether the mark is built with the thing that makes it contribute nothing, or with that
/// one thing restored.
///
/// [`Ink::Full`] is what makes this file's assertions measurements: three of the six kinds
/// are answered before any shader runs — an empty clip culls the command, a zero-area
/// rectangle produces a quad with no fragments, a zero-area outline produces no coverage —
/// so a floor of ink planted in a fragment shader cannot reach them, and "the target did
/// not change" would read the same if the fixture had drawn nothing in the first place.
/// The control asserts that each of the six *does* change the target when its one
/// nothing-making property is restored, which is the only way to tell an answer from an
/// absence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ink {
    /// The mark as this file's question asks for it: contributing nothing.
    None,
    /// The same mark with its shape or its opacity restored, and nothing else changed.
    Full,
}

/// Opaque magenta: chosen so that if any of these marks did reach the target it would be
/// visible against every part of [`backdrop`].
fn opaque() -> Paint {
    Paint::Solid(Color::new(1.0, 0.0, 1.0, 1.0))
}

/// A fill of the [`OVER`] triangle under a given clip, mask and paint — the one mark shape
/// four of the six kinds share, so that the only thing differing between them is the one
/// property each is named for.
fn solid(
    builder: &mut SceneBuilder,
    device: &mut Device,
    clip: Option<quorra_scene::ClipId>,
    mask: Option<quorra_scene::MaskId>,
    paint: Paint,
) -> Result<(), SceneError> {
    let outline = device
        .upload_outline(&triangle(
            OVER.min,
            Point::new(OVER.max.x, OVER.min.y),
            OVER.max,
        ))
        .expect("upload");
    builder.fill(
        outline,
        Affine::IDENTITY,
        FillRule::NonZero,
        paint,
        clip,
        BlendMode::Normal,
        Compose::SrcOver,
        mask,
    )
}

/// Append the mark that must contribute nothing (or, under [`Ink::Full`], its control).
///
/// Split along the clause's own seam — §11.6.4.2's shape and §11.6.4.4's opacity — because
/// that is the seam the whole file is about, and the two halves have different answers
/// inside a knockout group.
fn append_nothing(
    builder: &mut SceneBuilder,
    device: &mut Device,
    nothing: Nothing,
    ink: Ink,
) -> Result<(), SceneError> {
    match nothing {
        Nothing::ZeroAlphaFill | Nothing::ZeroAlphaImage | Nothing::TransparentSoftMask => {
            append_no_opacity(builder, device, nothing, ink)
        }
        _ => append_no_shape(builder, device, nothing, ink),
    }
}

/// The kinds whose **shape** is zero (§11.6.4.2), each by its own route into the target.
fn append_no_shape(
    builder: &mut SceneBuilder,
    device: &mut Device,
    nothing: Nothing,
    ink: Ink,
) -> Result<(), SceneError> {
    let full = ink == Ink::Full;
    match nothing {
        Nothing::EmptyClip => {
            // Two rectangles: disjoint under `Ink::None`, so the chain's intersection is
            // empty; overlapping under `Ink::Full`, so it admits the mark.
            let second = if full {
                Rect::new(Point::new(10.0, 10.0), Point::new(60.0, 60.0))
            } else {
                Rect::new(Point::new(40.0, 40.0), Point::new(60.0, 60.0))
            };
            let first = device
                .upload_outline(&common::scene::rect_outline(Rect::new(
                    Point::new(2.0, 2.0),
                    Point::new(20.0, 20.0),
                )))
                .expect("upload");
            let second = device
                .upload_outline(&common::scene::rect_outline(second))
                .expect("upload");
            let outer = builder.clip(first, Affine::IDENTITY, FillRule::NonZero, None)?;
            let chain = builder.clip(second, Affine::IDENTITY, FillRule::NonZero, Some(outer))?;
            solid(builder, device, Some(chain), None, opaque())
        }
        Nothing::ZeroAreaRect => builder.rect(
            Rect::new(
                Point::new(20.0, 20.0),
                Point::new(if full { 34.0 } else { 20.0 }, 44.0),
            ),
            Affine::IDENTITY,
            Color::new(1.0, 0.0, 1.0, 1.0),
            None,
            None,
        ),
        Nothing::ZeroAreaFill => {
            // Three collinear points enclose no region under either fill rule (§8.5.3.3);
            // moving the middle one off the line gives the same outline an area.
            let flat = device
                .upload_outline(&triangle(
                    Point::new(10.0, 20.0),
                    Point::new(30.0, if full { 44.0 } else { 20.0 }),
                    Point::new(50.0, 20.0),
                ))
                .expect("upload");
            builder.fill(
                flat,
                Affine::IDENTITY,
                FillRule::NonZero,
                opaque(),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
        }
        Nothing::ZeroAlphaFill | Nothing::ZeroAlphaImage | Nothing::TransparentSoftMask => {
            unreachable!("dispatched to append_no_opacity")
        }
    }
}

/// The kinds whose shape is 1 inside their own geometry and whose **opacity** is zero
/// (§11.6.4.4 for the fill's constant alpha, §11.6.4.2 for the image's rectangle,
/// §11.6.4.3 for the soft mask under Table 57's default alpha source flag).
fn append_no_opacity(
    builder: &mut SceneBuilder,
    device: &mut Device,
    nothing: Nothing,
    ink: Ink,
) -> Result<(), SceneError> {
    let alpha = if ink == Ink::Full { 1.0 } else { 0.0 };
    match nothing {
        Nothing::ZeroAlphaFill => solid(
            builder,
            device,
            None,
            None,
            Paint::Solid(Color::new(1.0, 0.0, 1.0, alpha)),
        ),
        Nothing::ZeroAlphaImage => {
            let image = device.upload_image(&white_image()).expect("upload");
            // §8.9.5.1 places every image on the unit square, so the placement is the
            // rectangle `OVER` written as an affine.
            builder.image(
                image,
                Affine {
                    a: OVER.max.x - OVER.min.x,
                    b: 0.0,
                    c: 0.0,
                    d: OVER.max.y - OVER.min.y,
                    e: OVER.min.x,
                    f: OVER.min.y,
                },
                alpha,
                ImageFilter::Nearest,
                None,
                BlendMode::Normal,
                None,
            )
        }
        Nothing::TransparentSoftMask => {
            // A mask group that marks nothing reduces to 0 everywhere (§11.5.2 with the
            // identity transfer); one that marks the page opaquely reduces to 1 there.
            // The mark's shape is the triangle either way — the mask is `qm`, not `fm`.
            let mask = builder.mask(MaskKind::Alpha, None, |mask| {
                if ink == Ink::Full {
                    mask.rect(
                        Rect::new(Point::new(0.0, 0.0), Point::new(64.0, 64.0)),
                        Affine::IDENTITY,
                        Color::new(1.0, 1.0, 1.0, 1.0),
                        None,
                        None,
                    )?;
                }
                Ok(())
            })?;
            solid(builder, device, None, Some(mask), opaque())
        }
        _ => unreachable!("dispatched to append_no_shape"),
    }
}

/// The group's body: its own content, then the mark that must contribute nothing.
fn body(
    builder: &mut SceneBuilder,
    device: &mut Device,
    nothing: Option<(Nothing, Ink)>,
) -> Result<(), SceneError> {
    group_content(builder, device)?;
    match nothing {
        Some((nothing, ink)) => append_nothing(builder, device, nothing, ink),
        None => Ok(()),
    }
}

/// The whole page: backdrop, then a group (or not) holding its content and, when `nothing`
/// is given, the mark that must contribute nothing.
fn page(device: &mut Device, context: Context, nothing: Option<(Nothing, Ink)>) -> Vec<u8> {
    let mut builder = SceneBuilder::new();
    backdrop(&mut builder, device).expect("valid backdrop");
    if context == Context::Root {
        body(&mut builder, device, nothing).expect("valid page");
    } else {
        builder
            .group(spec(context), |builder| body(builder, device, nothing))
            .expect("valid group");
    }
    let scene = builder.finish();
    render(device, &scene, SIZE, SIZE)
}

/// §11.4.6, NOTE 5: "A shape value of 0.0 (outside) leaves the previous group results
/// unchanged." No condition on the kind of group, so this holds in all four contexts —
/// and byte identity, not near-identity, is what "unchanged" means.
#[test]
fn a_mark_with_no_shape_leaves_the_target_byte_identical() {
    let mut device = device();
    // Every combination is reported, not only the first that fails: the four kinds reach
    // the target by four different routes — one is culled at encode, one is a coverage
    // byte, one an analytic area, one a sampled mask — so which of them broke is the
    // finding, and a bare `assert_eq!` in the loop would name only the earliest.
    let mut broken = Vec::new();
    for context in CONTEXTS {
        let without = page(&mut device, context, None);
        for nothing in NO_SHAPE {
            if page(&mut device, context, Some((nothing, Ink::None))) != without {
                broken.push(format!("{nothing:?} in {context:?}"));
            }
        }
    }
    assert!(
        broken.is_empty(),
        "§11.4.6 NOTE 5: a shape value of 0.0 leaves the previous group results unchanged; \
         these changed it: {broken:?}"
    );
}

/// The same for a mark whose shape is not zero but whose opacity is, **outside** a
/// knockout group. §11.3.6's compositing formula with a source alpha of 0 leaves both the
/// colour and the alpha of the backdrop, and §11.4.4's recurrence carries that through a
/// group unchanged.
#[test]
fn a_mark_with_no_opacity_leaves_the_target_byte_identical_outside_a_knockout_group() {
    let mut device = device();
    let mut broken = Vec::new();
    for context in [Context::Root, Context::Isolated, Context::NonIsolated] {
        let without = page(&mut device, context, None);
        for nothing in NO_OPACITY {
            if page(&mut device, context, Some((nothing, Ink::None))) != without {
                broken.push(format!("{nothing:?} in {context:?}"));
            }
        }
    }
    assert!(
        broken.is_empty(),
        "§11.3.6: a source alpha of 0 leaves the backdrop; these changed it: {broken:?}"
    );
}

/// **The discriminating half, and the reason the tests above are measurements rather than
/// coincidences.** Inside a knockout group the same zero-opacity mark is *not* a no-op:
/// §11.4.6 composites each element with the group's initial backdrop — transparent, for an
/// isolated knockout group — and replaces a shape-fraction of the accumulated group with
/// the result. The shape here is 1 inside the mark's path (§11.6.4.2), so the group is
/// left transparent where the mark covers, and the page's backdrop shows through.
///
/// Asserted as a difference from the *plain* group rather than against an absolute colour:
/// the two scenes differ in one boolean, so any difference between them is the knockout,
/// and the pixel probe then says the difference is in the direction the clause requires.
#[test]
fn a_mark_with_no_opacity_knocks_out_inside_a_knockout_group() {
    let mut device = device();
    // Inside `OVER` and inside `group_content`'s wedge: a pixel the knockout must clear.
    let (x, y) = (44_usize, 32_usize);
    let at = |pixels: &[u8]| {
        let i = (y * SIZE as usize + x) * 4;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    let mut bare = SceneBuilder::new();
    backdrop(&mut bare, &mut device).expect("valid backdrop");
    let bare_page = render(&mut device, &bare.finish(), SIZE, SIZE);
    for nothing in NO_OPACITY {
        let plain = page(&mut device, Context::Isolated, Some((nothing, Ink::None)));
        let knocked = page(&mut device, Context::Knockout, Some((nothing, Ink::None)));
        let bare = page(&mut device, Context::Knockout, None);
        assert_ne!(
            knocked, bare,
            "§11.4.6: a zero-opacity element of a knockout group replaces what it covers — {nothing:?}"
        );
        // The group's own content is what the knockout removes, so the plain group still
        // shows it and the knockout group does not.
        assert_ne!(
            at(&plain),
            at(&knocked),
            "§11.4.6 replaces where §11.4.4 composites — {nothing:?}"
        );
        // And what is left there is the page's backdrop alone: the group's contribution at
        // that pixel has been replaced by a transparent element composited with the
        // group's transparent initial backdrop, so the group adds nothing there.
        assert_eq!(
            at(&knocked),
            at(&bare_page),
            "§11.4.6: the knocked-out pixel is the page under the group — {nothing:?}"
        );
    }
}

/// **The control that makes every assertion above a measurement.** Each of the six kinds,
/// with its one nothing-making property restored and nothing else changed, must change the
/// target — otherwise "the target did not change" is a statement about a fixture that never
/// drew anything, which is the shape `HANDOVER.md` records as a determinism fixture that
/// does not overlap.
///
/// It matters here more than usual because the six are answered at four different depths:
/// an empty clip culls the command before any pass is recorded, a zero-area rectangle
/// produces a quad with no fragments, a zero-area outline produces no coverage byte, and
/// only the transparent soft mask and the two zero-opacity marks reach a fragment shader at
/// all. A defect planted in a shader can be seen by one of them and by none of the others,
/// which is exactly what happened when these gates were checked: an ink floor in
/// `coverage_at` failed `TransparentSoftMask` in all four contexts and left the other three
/// passing. This test is what stands behind those three.
#[test]
fn the_marks_this_file_asserts_are_invisible_are_visible_when_they_are_given_ink() {
    let mut device = device();
    let mut silent = Vec::new();
    for context in CONTEXTS {
        let without = page(&mut device, context, None);
        for nothing in NO_SHAPE.iter().chain(&NO_OPACITY) {
            if page(&mut device, context, Some((*nothing, Ink::Full))) == without {
                silent.push(format!("{nothing:?} in {context:?}"));
            }
        }
    }
    assert!(
        silent.is_empty(),
        "a mark whose shape and opacity are restored must reach the target, or this file \
         asserts nothing; these did not: {silent:?}"
    );
}

/// A group that marks nothing at all is the same "nothing" one level up, and §11.4.4's
/// Result step is where it could stop being one: the clause's own formulas remove the
/// backdrop's contribution and the composite puts it back, so a group with no elements
/// must leave the page byte-identical rather than nearly so.
#[test]
fn a_group_that_marks_nothing_leaves_the_target_byte_identical() {
    let mut device = device();
    let mut plain = SceneBuilder::new();
    backdrop(&mut plain, &mut device).expect("valid backdrop");
    let bare = render(&mut device, &plain.finish(), SIZE, SIZE);

    for context in [Context::Isolated, Context::Knockout, Context::NonIsolated] {
        let mut builder = SceneBuilder::new();
        backdrop(&mut builder, &mut device).expect("valid backdrop");
        builder
            .group(spec(context), |_| Ok(()))
            .expect("valid group");
        let with = render(&mut device, &builder.finish(), SIZE, SIZE);
        assert_eq!(
            with, bare,
            "§11.4.4: a group with no elements contributes no shape and no alpha — {context:?}"
        );
    }
}

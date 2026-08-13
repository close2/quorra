//! What a soft mask is **outside the part of the page its group marks** (ADR 0037).
//!
//! §11.5 makes a soft mask a transparency group rendered at device resolution and then
//! reduced; a group covers what it draws, so realising one at the whole target spends a
//! page's worth of memory on a mask that may mark a corner — 93 MB of `issue16287.pdf`'s
//! 291 at 4×. A mask realised at its plan's rectangle costs what it covers, and every
//! sampler of it then needs the value *elsewhere*.
//!
//! **That value is not zero and it is not the nearest edge texel.** It is what the
//! reduction writes for a fully transparent pixel, which is what a whole-target
//! realisation held out there: `transfer[0]` under §11.5.2's alpha rule, and the
//! transferred luminosity of the backdrop under §11.5.3's — so a luminosity mask over
//! white admits everything outside its group and one over the caller's default black
//! admits nothing. Getting that constant wrong is a plausible-looking wrong page, which
//! is the outcome §5 calls the worst one, so the expectations below come from the clause
//! and the tests hold the device to them on both sides of a mask's boundary.

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

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{Affine, Color, MaskKind, Point, Rect, Scene, SceneBuilder, Transfer};

/// 64 pixels wide: 64 × 4 bytes = 256, the buffer-copy row alignment.
const SIZE: u32 = 64;

/// Where the mask groups below mark, in device pixels. Well inside the page, so that
/// "outside the mask" is a large region and the boundary is nowhere near the target's.
const MARKED: f32 = 16.0;

fn device() -> Device {
    device_with(&Options {
        adapter: Some("llvmpipe".into()),
        ..Options::default()
    })
}

fn device_with(options: &Options) -> Device {
    Device::headless(options).expect("llvmpipe is present wherever this suite runs")
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

/// The alpha of one pixel of a straight-alpha RGBA readback.
fn alpha(pixels: &[u8], x: u32, y: u32) -> u8 {
    pixels[((y * SIZE + x) * 4 + 3) as usize]
}

/// §11.5's mask value where the mask's group marks nothing, derived from the clause.
///
/// The group contributes nothing there, so §11.5.2 ("the mask value … derived from the
/// alpha of the group") derives 0, and §11.5.3 — the group composited with *a fully
/// opaque backdrop of a specified colour*, then the luminosity of the result — derives
/// the luminosity of that backdrop alone, since a fully transparent source leaves the
/// backdrop unchanged. §11.6.5.1's transfer function maps whichever byte results.
fn transparent_mask_value(kind: MaskKind, transfer: Option<&Transfer>) -> u8 {
    let derived = match kind {
        MaskKind::Alpha => 0,
        MaskKind::Luminosity { backdrop } => {
            let luminosity =
                0.30_f32.mul_add(backdrop.r, 0.59_f32.mul_add(backdrop.g, 0.11 * backdrop.b));
            (luminosity * 255.0).round().clamp(0.0, 255.0) as u8
        }
    };
    transfer.map_or(derived, |t| t.apply(derived))
}

/// A transfer that is visibly not the identity, endpoints included — it turns an alpha
/// mask's "admits nothing" into "admits everything", which is the difference between
/// carrying the constant and assuming zero.
fn inverting_transfer() -> Transfer {
    let mut table = [0_u8; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        *slot = 255 - (i as u8);
    }
    Transfer(table)
}

/// The four cases: both rules, each with and without a transfer, and a luminosity
/// backdrop that is not black (the default that hides the term).
fn cases() -> Vec<(MaskKind, Option<Transfer>)> {
    vec![
        (MaskKind::Alpha, None),
        (MaskKind::Alpha, Some(inverting_transfer())),
        (
            MaskKind::Luminosity {
                backdrop: Color::new(0.0, 0.0, 0.0, 1.0),
            },
            None,
        ),
        (
            MaskKind::Luminosity {
                backdrop: Color::new(0.25, 0.5, 0.75, 1.0),
            },
            Some(inverting_transfer()),
        ),
    ]
}

/// A page of opaque white under a mask whose group marks the `MARKED` square opaquely.
/// The readback's alpha *is* the mask value at each pixel.
fn masked_page(kind: MaskKind, transfer: Option<Transfer>, mask_marks: bool) -> Scene {
    let mut builder = SceneBuilder::new();
    let mask = builder
        .mask(kind, transfer, |body| {
            if !mask_marks {
                return Ok(());
            }
            body.rect(
                Rect::new(Point::new(0.0, 0.0), Point::new(MARKED, MARKED)),
                Affine::IDENTITY,
                Color::new(1.0, 1.0, 1.0, 1.0),
                None,
                None,
            )
        })
        .expect("valid mask");
    builder
        .rect(
            Rect::new(Point::new(0.0, 0.0), Point::new(SIZE as f32, SIZE as f32)),
            Affine::IDENTITY,
            Color::new(1.0, 1.0, 1.0, 1.0),
            None,
            Some(mask),
        )
        .expect("valid masked rect");
    builder.finish()
}

/// Outside the rectangle a mask's group marks, the mask is the reduction of a fully
/// transparent pixel — for every rule, backdrop and transfer.
///
/// This is the value a mask realised at its plan's rectangle can no longer read out of a
/// texture, so it is carried as a constant instead; the test is what holds that constant
/// to the same clause the reduction implements.
#[test]
fn a_mask_is_its_transparent_reduction_where_its_group_marks_nothing() {
    let mut device = device();
    for (kind, transfer) in cases() {
        let pixels = render(&mut device, &masked_page(kind, transfer.clone(), true));
        let expected = transparent_mask_value(kind, transfer.as_ref());
        for (x, y) in [(40, 40), (SIZE - 1, SIZE - 1), (0, 40), (40, 0)] {
            assert_eq!(
                alpha(&pixels, x, y),
                expected,
                "({x}, {y}) is outside the mask group under {kind:?} \
                 (transfer: {})",
                transfer.is_some(),
            );
        }
    }
}

/// And inside it, the mask is its group's own reduction — so the boundary of a mask's
/// rectangle is a boundary of *values*, not an artefact of how big the texture is.
///
/// An opaque white mark reduces to 255 under the alpha rule and to the luminosity of
/// white — 255 again, the clause's coefficients summing to 1 — under the other, before
/// the transfer.
#[test]
fn a_mask_is_its_groups_reduction_inside_the_rectangle_it_marks() {
    let mut device = device();
    for (kind, transfer) in cases() {
        let pixels = render(&mut device, &masked_page(kind, transfer.clone(), true));
        let expected = transfer.as_ref().map_or(255, |t| t.apply(255));
        for (x, y) in [(0, 0), (8, 8), (MARKED as u32 - 1, MARKED as u32 - 1)] {
            assert_eq!(
                alpha(&pixels, x, y),
                expected,
                "({x}, {y}) is inside the mask group under {kind:?} \
                 (transfer: {})",
                transfer.is_some(),
            );
        }
        // The step is exactly at the group's edge, in both axes.
        assert_ne!(
            alpha(&pixels, MARKED as u32 - 1, 0),
            alpha(&pixels, MARKED as u32, 0),
            "the mask changes value at its group's right edge under {kind:?}",
        );
        assert_ne!(
            alpha(&pixels, 0, MARKED as u32 - 1),
            alpha(&pixels, 0, MARKED as u32),
            "the mask changes value at its group's bottom edge under {kind:?}",
        );
    }
}

/// What the sizing is for: a page whose mask covers a corner is priced for that corner,
/// and that price is what the frame then allocates.
///
/// The arithmetic is machine-independent, so it may be asserted exactly. On this 64 × 64
/// target with a mask group over 16 × 16:
///
/// | | bytes |
/// |---|---:|
/// | the root's pair, `64 × 64 × 4 × 2` — always the target's, because the root *is* it | 32 768 |
/// | the mask group's pair, `16 × 16 × 4 × 2`, released before the root draws | 2 048 |
/// | the reduced mask, one R8 byte per texel of the same rectangle | 256 |
/// | | **35 072** |
///
/// Where a mask realised at the whole target adds `64 × 64` = 4 096 instead of 256 and
/// renders its group into a target-sized pair rather than a 16 × 16 one, for 68 608.
/// A budget of 35 072 draws this page; one byte less refuses it, naming both numbers (§5).
///
/// The exactness is the point twice over. Before this sizing the two halves disagreed:
/// a mask group's plan was *priced* at its own bounds and *realised* at the target, so
/// the budget check passed a frame that then allocated sixteen times what it promised —
/// count-then-allocate counting one thing and allocating another.
///
/// `issue16287.pdf` at 4× is the same sum on a 2 448 × 9 504 page: four masks, 93 MB.
#[test]
fn a_mask_over_a_corner_is_priced_for_the_corner() {
    let scene = masked_page(MaskKind::Alpha, None, true);
    let viewport = Viewport::full(SIZE, SIZE, Affine::IDENTITY);
    let budgeted = |bytes: u64| {
        device_with(&Options {
            adapter: Some("llvmpipe".into()),
            max_frame_bytes: bytes,
            ..Options::default()
        })
    };

    let mut exact = budgeted(35_072);
    exact
        .render(&scene, &viewport, Target::Readback)
        .expect("a mask over a sixteenth of the page is priced for a sixteenth of it");

    let mut short = budgeted(35_071);
    let refused = short
        .render(&scene, &viewport, Target::Readback)
        .expect_err("one byte short of the frame's own arithmetic");
    assert!(
        matches!(
            refused,
            quorra_gpu::RenderError::FrameBudgetExceeded {
                needed: 35_072,
                budget: 35_071,
            }
        ),
        "the refusal names what overflowed and by how much, got {refused:?}"
    );
}

/// A mask whose group marks **nothing at all** is its transparent reduction everywhere —
/// the degenerate rectangle, which a plan sized to its bounds has no pixels of.
///
/// A real case rather than a contrived one: a mask group whose content is entirely
/// clipped away, or off the page, arrives here. Under the alpha rule with no transfer it
/// admits nothing and the page is blank, which is a legitimate frame (§5) and not the
/// blank one that means a defect.
#[test]
fn a_mask_whose_group_marks_nothing_is_transparent_everywhere() {
    let mut device = device();
    for (kind, transfer) in cases() {
        let pixels = render(&mut device, &masked_page(kind, transfer.clone(), false));
        let expected = transparent_mask_value(kind, transfer.as_ref());
        for (x, y) in [(0, 0), (1, 1), (32, 32), (SIZE - 1, SIZE - 1)] {
            assert_eq!(
                alpha(&pixels, x, y),
                expected,
                "({x}, {y}) under an unmarked mask group, {kind:?} \
                 (transfer: {})",
                transfer.is_some(),
            );
        }
    }
}

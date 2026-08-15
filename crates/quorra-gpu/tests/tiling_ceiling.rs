//! The two shapes that make a frame ask for more coverage sheet than an adapter has,
//! stated as measurements a test can take through the public API.
//!
//! `doc/notes-tiling-ceiling.md` is the round these witness. Two pages of the caller's
//! corpus refuse with [`RenderError::ScratchExhausted`] from 2× magnification upwards,
//! and the two causes are different — which is the whole finding, and is why this file
//! has two halves:
//!
//! 1. **A residue clip does not bound the tile its mark asks for.** A page-sized mark
//!    under an 18 × 19-pixel curved clip is charged a page-sized coverage tile, because
//!    a non-rectangular link contributes nothing to the resolved clip *rectangle* that
//!    `coverage_tile` intersects with. On `bug1703683_page2_reduced.pdf` at 4× that is
//!    1.008 GB of coverage where the chains admit 2.3 MB of it.
//! 2. **The sheet's ceiling is a sum of tile heights, not an area.** Tall tiles each
//!    take their own shelf, so the sheet runs out of *height* while both its width and
//!    the frame's byte budget are still free.
//!
//! **How a tile's size is read from outside the crate.** The frame budget is charged
//! tile by tile before anything is allocated, and
//! [`RenderError::FrameBudgetExceeded`] names the running total — so a device whose
//! budget sits between two candidate tile sizes says, by refusing or not refusing, which
//! side of it a tile fell. That is the only instrument the public API has for this, and
//! naming it here is half the point of the file: `Counters` counts tiles and never says
//! how large one was.

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
    Affine, BlendMode, Color, Compose, FillRule, OutlineId, Paint, Point, Scene, SceneBuilder,
    Segment,
};

fn device_with(max_frame_bytes: u64) -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        max_frame_bytes,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

/// A closed curve — not a rectangle under any transform, so a clip built on it stays a
/// residue link and a fill built on it takes the path lane.
fn blob(device: &mut Device, rx: f32, ry: f32) -> OutlineId {
    let mut path = vec![Segment::MoveTo(Point::new(-rx, 0.0))];
    for step in 0..8_u8 {
        let from = f32::from(step) / 8.0 * std::f32::consts::TAU;
        let to = f32::from(step + 1) / 8.0 * std::f32::consts::TAU;
        let at = |angle: f32| Point::new(rx * angle.cos(), ry * angle.sin());
        let (a, b) = (at(from), at(to));
        path.push(Segment::CubicTo {
            c1: Point::new(a.x + (b.x - a.x) * 0.35, a.y + (b.y - a.y) * 0.1),
            c2: Point::new(a.x + (b.x - a.x) * 0.65, a.y + (b.y - a.y) * 0.9),
            to: b,
        });
    }
    path.push(Segment::Close);
    device.upload_outline(&path).expect("an outline")
}

/// An axis-aligned rectangle outline, which `ResourceStore` recognises and a clip built
/// on it collapses into the resolved rectangle (ADR 0007).
fn box_outline(device: &mut Device, rx: f32, ry: f32) -> OutlineId {
    device
        .upload_outline(&[
            Segment::MoveTo(Point::new(-rx, -ry)),
            Segment::LineTo(Point::new(rx, -ry)),
            Segment::LineTo(Point::new(rx, ry)),
            Segment::LineTo(Point::new(-rx, ry)),
            Segment::Close,
        ])
        .expect("an outline")
}

/// What a scene costs this device, as the frame budget sees it: `None` when the frame
/// draws, and the bytes it asked for when the budget refused it.
fn charged_or_drawn(device: &mut Device, scene: &Scene, width: u32, height: u32) -> Option<u64> {
    match device.render(
        scene,
        &Viewport::full(width, height, Affine::IDENTITY),
        Target::Readback,
    ) {
        Err(RenderError::FrameBudgetExceeded { needed, .. }) => Some(needed),
        Err(other) => panic!("expected either a drawn frame or the byte budget, got {other}"),
        Ok(_) => None,
    }
}

/// One page-sized fill, optionally under `clip`.
fn page_fill(
    builder: &mut SceneBuilder,
    mark: OutlineId,
    at: Point,
    clip: Option<quorra_scene::ClipId>,
) {
    builder
        .fill(
            mark,
            Affine::translate(at.x, at.y),
            FillRule::NonZero,
            Paint::Solid(Color::new(0.1, 0.2, 0.9, 1.0)),
            clip,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("a fill");
}

/// **A residue clip does not bound the coverage tile its mark asks for; a rectangular
/// one does.**
///
/// The tile is `shape ∩ clip.rect ∩ target` (`encode.rs`'s `coverage_tile`), and a
/// non-rectangular link never reaches `clip.rect` — `ClipResolver::resolve` keeps it in
/// `residues` and leaves the rectangle alone. So the two scenes below differ only in
/// whether the clip on a page-sized mark is a curve or a rectangle of the same extent,
/// and a budget with room for four pages of coverage draws one and refuses the other.
///
/// Outside the curve the chain's coverage is zero and the product this tile carries is
/// zero with it, so those bytes are rasterised, packed, uploaded and sampled to no
/// effect. That is what `bug1703683_page2_reduced.pdf` does 130 times a frame: at 4× its
/// chains admit 2.3 MB and its tiles ask for 1.008 GB of coverage.
///
/// **The budget is 2 MB and the two legs are three times either side of it.** A frame
/// charges more than its tiles — the accumulator this page needs is a 256 × 256
/// half-float layer, 524 288 bytes — so the separation is made by weight of numbers
/// rather than by a threshold cut fine: sixty-four tiles of at most 256 bytes plus that
/// accumulator is under 0.6 MB, and sixty-four tiles of 65 536 bytes is 4 MB.
#[test]
fn a_residue_clip_does_not_bound_the_tile_its_mark_asks_for() {
    const SIDE: u32 = 256;
    const MARKS: u8 = 64;
    // Three times over what the rectangular leg needs, half what the curved leg does.
    const BUDGET: u64 = 2_000_000;
    let page = u64::from(SIDE) * u64::from(SIDE);

    let scene_under = |device: &mut Device, curved: bool| {
        let mark = blob(device, 400.0, 400.0);
        let clip_outline = if curved {
            blob(device, 6.0, 6.0)
        } else {
            box_outline(device, 6.0, 6.0)
        };
        let centre = Point::new(SIDE as f32 / 2.0, SIDE as f32 / 2.0);
        let mut builder = SceneBuilder::new();
        for i in 0..MARKS {
            let clip = builder
                .clip(
                    clip_outline,
                    Affine::translate(centre.x + f32::from(i) * 0.25, centre.y),
                    FillRule::NonZero,
                    None,
                )
                .expect("a clip");
            page_fill(&mut builder, mark, centre, Some(clip));
        }
        builder.finish()
    };

    let mut device = device_with(BUDGET);
    let boxed = scene_under(&mut device, false);
    let curved = scene_under(&mut device, true);
    let under_rect = charged_or_drawn(&mut device, &boxed, SIDE, SIDE);
    let under_curve = charged_or_drawn(&mut device, &curved, SIDE, SIDE);

    assert_eq!(
        under_rect, None,
        "a rectangular clip 12 pixels across bounds the tile, so {MARKS} marks under one \
         cost far less than four pages"
    );
    assert!(
        under_curve.is_some_and(|bytes| bytes >= 4 * page),
        "a curved clip of the same extent left each tile the size of the page: \
         {under_curve:?} against a page of {page}"
    );
}

/// **A frame is refused for its sheet's height with its byte budget untouched.**
///
/// `ScratchPacker::reserve` seats a tile on an existing shelf only when that shelf is at
/// least as tall as the tile and no more than twice as tall, so a run of *increasing*
/// tile heights opens a shelf every time — and the sheet is as tall as the shelves it
/// opened, whatever area they hold. Eight thin tiles here overflow the adapter's 16 384
/// rows while holding about 175 000 texels, which is 0.07 % of the default frame budget.
///
/// **This is a stand-in for the mechanism and not for the magnitude.** The corpus's two
/// refusing pages reach the same sum a different way — 7 shelves of 3 168 rows and 2 of
/// 7 710 — because their tiles are page-sized and a shelf fills in width before the
/// sheet is tall. What both share, and what this pins, is that the ceiling is a **sum of
/// shelf heights** and that the byte budget is a separate, independently reached limit:
/// `RenderError`'s two variants are not interchangeable, and a caller told the wrong one
/// cannot diagnose the page.
#[test]
fn a_frame_is_refused_for_the_sheets_height_with_its_bytes_untouched() {
    let mut device = device_with(Options::default().max_frame_bytes);
    // `encode.rs` sizes `ScratchPacker` from the adapter's texture dimension in both
    // axes, and `Limits::max_target_size` is that number.
    let ceiling = device.limits().max_target_size;
    // Eight of these overflow the ceiling and five do not, on any adapter.
    let tall = ceiling / 7;
    let wide = 8;

    // Distinct outlines of *increasing* height, each placed once: the atlas declines a
    // tile it cannot reuse (ADR 0029), so every one of these reaches the sheet, and no
    // later tile fits any earlier tile's shelf.
    let marks: Vec<OutlineId> = (0..8_u8)
        .map(|i| {
            blob(
                &mut device,
                wide as f32 / 2.0,
                tall as f32 / 2.0 + f32::from(i),
            )
        })
        .collect();

    let height = tall + 32;
    let scene = |count: usize| {
        let mut builder = SceneBuilder::new();
        for mark in marks.iter().take(count) {
            page_fill(
                &mut builder,
                *mark,
                Point::new(wide as f32 / 2.0, height as f32 / 2.0),
                None,
            );
        }
        builder.finish()
    };

    let refused = device.render(
        &scene(8),
        &Viewport::full(wide, height, Affine::IDENTITY),
        Target::Readback,
    );
    match refused {
        Err(RenderError::ScratchExhausted { limit }) => assert_eq!(
            limit, ceiling,
            "the refusal names the adapter's own limit, which is the only number about \
             this budget a caller can reach"
        ),
        Err(other) => panic!("eight shelves of {tall} rows must not fit {ceiling}: {other}"),
        Ok(_) => panic!("eight shelves of {tall} rows must not fit {ceiling}"),
    }

    // Five shelves: 5/7 of the ceiling, which fits — same marks, same width, same lane.
    device
        .render(
            &scene(5),
            &Viewport::full(wide, height, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("five shelves of the same tiles fit the same sheet");
}

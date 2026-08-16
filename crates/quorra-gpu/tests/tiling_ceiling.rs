//! The two shapes that make a frame ask for more coverage sheet than an adapter has,
//! stated as measurements a test can take through the public API.
//!
//! `doc/notes-tiling-ceiling.md` is the round these witness and `doc/notes-tiling-bound.md`
//! the round that acted on it. Two pages of the caller's corpus refused with
//! [`RenderError::ScratchExhausted`] from 2× magnification upwards, and the two causes
//! were different — which is the whole finding, and is why this file has two halves:
//!
//! 1. **A residue clip did not bound the tile its mark asks for.** A page-sized mark
//!    under an 18 × 19-pixel curved clip was charged a page-sized coverage tile, because
//!    a non-rectangular link contributes nothing to the resolved clip *rectangle*.
//!    On `bug1703683_page2_reduced.pdf` at 4× that was 1.008 GB of coverage where the
//!    chains admit 2.3 MB of it. **ADR 0057 closed it**, and the first test below is that
//!    finding inverted: the tile is now bounded by the chain's own device box, and the
//!    curved leg costs what the rectangular one does.
//! 2. **The sheet's ceiling is a sum of tile heights, not an area.** Tall tiles each
//!    take their own shelf, so the sheet runs out of *height* while both its width and
//!    the frame's byte budget are still free. That one stands, and the second test holds
//!    it — now with the refusal's own account of the sheet it met.
//!
//! **How a tile's size is read from outside the crate.** Two ways since ADR 0057, and
//! the file uses both. [`Counters::coverage`] prices a drawn frame's sheet — the texels
//! its tiles hold, which is the number the bound moves and which `tiles` alone could
//! never show. And the frame budget is still charged tile by tile before anything is
//! allocated, so [`RenderError::FrameBudgetExceeded`] names the running total for a
//! frame that has no counters because it was refused.

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

use quorra_gpu::{Counters, CoverageSheet, Device, Options, RenderError, Target, Viewport};
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

/// The same device with an atlas too small to hold any of this file's marks.
///
/// **The subject here is the scratch sheet**, and a large mark repeated many times is
/// exactly what the glyph lane exists to take off it: with the default budget the
/// rectangular leg below cached one 800 × 800 tile and placed *no* tile on the sheet at
/// all, so the two legs were being compared through different lanes. Sixty-four KiB
/// holds nothing this file uploads, which puts both legs on the path lane where the
/// comparison means something.
fn device_without_atlas(max_frame_bytes: u64) -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        max_frame_bytes,
        atlas_budget: 64 * 1024,
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

/// What a scene costs this device: the drawn frame's counters, or the bytes the frame
/// budget refused it for.
fn counted_or_charged(
    device: &mut Device,
    scene: &Scene,
    width: u32,
    height: u32,
) -> Result<Counters, u64> {
    match device.render(
        scene,
        &Viewport::full(width, height, Affine::IDENTITY),
        Target::Readback,
    ) {
        Err(RenderError::FrameBudgetExceeded { needed, .. }) => Err(needed),
        Err(other) => panic!("expected either a drawn frame or the byte budget, got {other}"),
        Ok(frame) => Ok(frame.counters()),
    }
}

/// The coverage a scene that must draw actually cost, with the sheet's own two
/// invariants checked wherever this is called: the sheet holds what its tiles hold, and
/// the two ways to count a tile agree.
fn coverage_of(device: &mut Device, scene: &Scene, width: u32, height: u32) -> CoverageSheet {
    let counters = counted_or_charged(device, scene, width, height)
        .unwrap_or_else(|needed| panic!("this scene must draw; the budget refused {needed} bytes"));
    let sheet = counters.coverage;
    assert_eq!(
        sheet.tiles, counters.tiles,
        "the sheet's tile count and Counters::tiles are one number reached two ways"
    );
    assert!(
        u64::from(sheet.width) * u64::from(sheet.height) >= sheet.texels,
        "a sheet of {sheet} cannot hold more texels than it has"
    );
    sheet
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

/// **A residue clip bounds the coverage tile its mark asks for, as a rectangular one
/// does** (ADR 0057).
///
/// The tile is `shape ∩ clip ∩ target`, and `clip` is `ResolvedClip::mark_bounds` — the
/// rectangular links' intersection held further by the box the residue links' control
/// hulls trace. Before ADR 0057 a non-rectangular link reached neither: `ClipResolver`
/// kept it in `residues` and left the rectangle alone, so a page-sized mark under a
/// twelve-pixel curve was charged a page-sized tile.
///
/// Outside the curve the chain's coverage is zero and the product the tile carries is
/// zero with it, so those bytes were rasterised, packed, uploaded and sampled to no
/// effect. That is what `bug1703683_page2_reduced.pdf` did 130 times a frame: at 4× its
/// chains admit 2.3 MB and its tiles asked for 1.008 GB of coverage.
///
/// **The two legs are the same scene with one clip outline exchanged**, and the
/// assertion is that they now cost the same coverage rather than differing by the ratio
/// of a page to a clip. The curved clip's bound is its *control hull*, which for this
/// blob is the circle of radius 6 its control points are convex combinations of — so the
/// two legs' tiles agree to the pixel of rounding, and the factor asserted below (2×) is
/// slack for that rounding rather than a threshold anybody tuned.
///
/// **The budget is 2 MB**, which is three times over what either leg now needs and half
/// what the unbounded curved leg took: a frame charges more than its tiles — the
/// accumulator this page needs is a 256 × 256 half-float layer, 524 288 bytes — and
/// sixty-four tiles of 65 536 texels is 4 MB on top of it.
#[test]
fn a_residue_clip_bounds_the_tile_its_mark_asks_for() {
    const SIDE: u32 = 256;
    const MARKS: u8 = 64;
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

    let mut device = device_without_atlas(BUDGET);
    let boxed = scene_under(&mut device, false);
    let curved = scene_under(&mut device, true);
    let under_rect = coverage_of(&mut device, &boxed, SIDE, SIDE);
    let under_curve = coverage_of(&mut device, &curved, SIDE, SIDE);

    assert_eq!(
        (under_rect.tiles, under_curve.tiles),
        (u32::from(MARKS), u32::from(MARKS)),
        "both legs place one coverage tile per mark; only their size was ever in question"
    );
    assert!(
        under_curve.texels <= 2 * under_rect.texels,
        "a curved clip must bound its mark's tile the way a rectangle of the same extent \
         does: {under_curve} against {under_rect}"
    );
    assert!(
        under_curve.texels < page,
        "{MARKS} marks under a twelve-pixel curve must cost less coverage than one page \
         of it, and cost {} pages before ADR 0057: {under_curve}",
        u64::from(MARKS)
    );
}

/// **The bounded tile still draws every pixel the chain admits** (ADR 0057).
///
/// The cheap half of a bound is making a tile smaller; the half that has to be checked in
/// the pixels is that nothing left the picture with it. `mark_bounds` uses a residue
/// link's **control hull**, which contains the curve by the convex-hull property of
/// Béziers, so the pixels it removes are pixels the chain's coverage is zero at. A box
/// derived one step too tight would not fail the counter assertions above at all — it
/// would draw a *smaller mark*, which only a picture can see.
///
/// The two scenes are chosen so that the answer is derivable rather than compared against
/// a stored image. A mark that covers the whole target with full coverage, multiplied by
/// the chain's coverage `c`, is `(255·c + 127) / 255 = c` exactly — so **a huge blob under
/// a curve clip must draw the same picture as that curve filled on its own**, to the byte,
/// and the tolerance below is the 1-of-255 that ADR 0049 measured between one region and a
/// tile cut out of it (`f32` addition is not associative and the two tiles are different
/// rectangles).
#[test]
fn a_bounded_tile_draws_every_pixel_the_chain_admits() {
    const SIDE: u32 = 256;
    let centre = Point::new(SIDE as f32 / 2.0, SIDE as f32 / 2.0);
    let mut device = device_without_atlas(Options::default().max_frame_bytes);
    let shape = blob(&mut device, 60.0, 60.0);
    // A **rectangle** four times the target, and not another blob: this file's blob is a
    // self-overlapping loop whose winding cancels at its own centre, so it is a band
    // rather than a disc and could not stand in for "covers every pixel". A rectangular
    // outline under a residue clip cannot take ADR 0007's analytic lane — a residue has
    // nowhere to go there — so it rasterises through the same `coverage_tile` every other
    // clipped mark does, which is the path under test.
    let mark = box_outline(&mut device, 400.0, 400.0);

    let alone = {
        let mut builder = SceneBuilder::new();
        page_fill(&mut builder, shape, centre, None);
        builder.finish()
    };
    let clipped = {
        let mut builder = SceneBuilder::new();
        let clip = builder
            .clip(
                shape,
                Affine::translate(centre.x, centre.y),
                FillRule::NonZero,
                None,
            )
            .expect("a clip");
        page_fill(&mut builder, mark, centre, Some(clip));
        builder.finish()
    };

    let raster = |device: &mut Device, scene: &Scene| {
        device
            .render(
                scene,
                &Viewport::full(SIDE, SIDE, Affine::IDENTITY),
                Target::Readback,
            )
            .expect("both scenes are far inside the default budget")
            .into_raster()
            .expect("a Readback frame carries its pixels")
    };
    let expected = raster(&mut device, &alone);
    let drawn = raster(&mut device, &clipped);

    let inked = expected
        .pixels()
        .iter()
        .skip(3)
        .step_by(4)
        .filter(|a| **a > 250)
        .count();
    assert!(
        inked > 1_000,
        "the control must actually draw the curve, or this proves nothing: {inked} opaque \
         pixels"
    );
    let worst = expected
        .pixels()
        .iter()
        .zip(drawn.pixels())
        .map(|(a, b)| u16::from(*a).abs_diff(u16::from(*b)))
        .max()
        .unwrap_or(0);
    assert!(
        worst <= 1,
        "a mark bounded by its chain's box must draw what the chain admits; the worst \
         channel differs by {worst} of 255 from the same curve filled unclipped"
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
///
/// **And the refusal accounts for the frame that met the wall** (ADR 0057). A refused
/// frame has no `Counters`, so until the variant carried the sheet it had reached, every
/// number in `doc/notes-tiling-ceiling.md` §1 had to be obtained by patching the crate:
/// `limit` alone is a property of this adapter and says nothing about the page. The
/// assertions below are the three questions that were unanswerable — which axis
/// overflowed, by how much, and how far the byte budget was from mattering.
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
        Err(RenderError::ScratchExhausted {
            limit,
            sheet,
            tile_width,
            tile_height,
        }) => {
            assert_eq!(limit, ceiling, "the refusal names the adapter's own limit");
            // Which axis, and by how much. Every mark here is `wide` across, so the
            // sheet's width is one tile and never near the wall; its height is the sum
            // of the shelves it opened, and the tile that did not fit is what took it
            // over.
            assert_eq!(
                (sheet.width, tile_width),
                (wide, wide),
                "one tile per shelf, all of them {wide} across: {sheet}"
            );
            assert!(
                sheet.height <= limit && sheet.height + tile_height > limit,
                "a sheet of {sheet} met the wall at {limit} with a \
                 {tile_width}x{tile_height} tile still to place"
            );
            // How far the byte budget was from mattering — the half of this test that
            // was an inference before the refusal carried the sheet.
            assert!(
                sheet.tiles >= 1
                    && sheet.texels >= u64::from(sheet.tiles) * u64::from(wide) * u64::from(tall),
                "every seated tile is at least {wide}x{tall}: {sheet}"
            );
            assert!(
                sheet.texels * 100 < Options::default().max_frame_bytes,
                "this sheet overflows on rows while holding under a hundredth of the \
                 frame budget: {sheet}"
            );
        }
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

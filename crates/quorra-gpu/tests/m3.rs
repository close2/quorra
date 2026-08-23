//! The M3 harness: rectangular clips resolved analytically, the distinct-region
//! counter, and the page-shaped fixtures whose absence let real failures hide.
//!
//! The two numbers this file exists around, both the caller's: its page 6 states one
//! clipping rectangle **303 times** and must count as **one** region (the region is
//! the key, never the identifier — its ADR 0132 lesson); and its worst page holds
//! **3 608 clip chains**, which must resolve within the ordinary budgets. And its
//! trap 12b: *a suite of small scenes tests small scenes* — so the collapse fixture
//! here is a full page of clipped content at a real window scale, not a toy.

// Test-file lint policy as in m1.rs; the casts index bounded page geometry.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Device, Options, RenderError, Target, Viewport};
use quorra_scene::{
    Affine, ClipId, Color, FillRule, OutlineId, Point, Rect, Scene, SceneBuilder, Segment,
};

mod common;

use common::bound::{Reference, disagreement};
use common::headless::device;
use common::scene::rect_outline;

/// The reference for clipped rectangles: ADR 0005's coverage and compositing rule
/// with ADR 0007's clip resolution — chains intersect to one rectangle, applied by
/// intersection before coverage.
///
/// It counts its stores, which is what [`disagreement`] reads the bound at each pixel
/// from — see the note above [`three_hundred_three_identical_clips_collapse_to_one_region`]
/// for why a clip suite in particular wants that rather than a constant.
fn cpu_reference(scene: &Scene, viewport: &Viewport<'_>, clip_rects: &[Rect]) -> Reference {
    let width = viewport.width as usize;
    let height = viewport.height as usize;
    let mut target = vec![[0_u8; 4]; width * height];
    let mut stores = vec![0_i32; width * height];
    for command in scene.commands() {
        let quorra_scene::Command::Rect {
            rect,
            transform,
            color,
            clip,
            mask: _,
        } = *command
        else {
            panic!("m3 scenes are rectangle-only");
        };
        let to_device = transform.then(viewport.transform);
        let p0 = to_device.apply(rect.min);
        let p1 = to_device.apply(rect.max);
        let mut device_rect = Rect::new(
            Point::new(p0.x.min(p1.x), p0.y.min(p1.y)),
            Point::new(p0.x.max(p1.x), p0.y.max(p1.y)),
        );
        if let Some(ClipId(id)) = clip {
            device_rect = device_rect.intersection(clip_rects[id as usize]);
        }
        if device_rect.is_empty() {
            continue;
        }
        let premul = [
            color.r * color.a,
            color.g * color.a,
            color.b * color.a,
            color.a,
        ];
        // Only the pixels the rectangle touches: the reference stays O(area), which
        // is what lets it check a full page at a window scale in test time.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (y0, y1, x0, x1) = (
            (device_rect.min.y.floor().max(0.0) as usize).min(height),
            (device_rect.max.y.ceil().max(0.0) as usize).min(height),
            (device_rect.min.x.floor().max(0.0) as usize).min(width),
            (device_rect.max.x.ceil().max(0.0) as usize).min(width),
        );
        for y in y0..y1 {
            for x in x0..x1 {
                let (px, py) = (x as f32, y as f32);
                let extent_x =
                    (device_rect.max.x.min(px + 1.0) - device_rect.min.x.max(px)).max(0.0);
                let extent_y =
                    (device_rect.max.y.min(py + 1.0) - device_rect.min.y.max(py)).max(0.0);
                let coverage = extent_x * extent_y;
                if coverage <= 0.0 {
                    continue;
                }
                let dst = &mut target[y * width + x];
                stores[y * width + x] += 1;
                let src_a = premul[3] * coverage;
                for channel in 0..4 {
                    let src = premul[channel] * coverage;
                    let dst_f = f32::from(dst[channel]) / 255.0;
                    let out = src + dst_f * (1.0 - src_a);
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    {
                        dst[channel] = (out.clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                }
            }
        }
    }
    let mut out = Vec::with_capacity(width * height * 4);
    for pixel in target {
        let a = pixel[3];
        if a == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            for channel in &pixel[..3] {
                let straight = (u32::from(*channel) * 255 + u32::from(a) / 2) / u32::from(a);
                #[allow(clippy::cast_possible_truncation)]
                out.push(straight.min(255) as u8);
            }
            out.push(a);
        }
    }
    Reference {
        pixels: out,
        stores,
    }
}

/// The page-6 shape at a window scale: 303 clip identifiers over one identical
/// region, thousands of clipped fills — the counter says **1**, and the pixels agree
/// with the reference. A full page at a real size, per trap 12b.
///
/// # Where this gate's bound comes from
///
/// It is [`common::bound::bound_at`], read at each pixel, and it replaced a
/// `const UNORM_TOLERANCE: i32 = 2` on 2026-08-23 (ADR 0077). Three facts about the
/// constant, all measured on this fixture rather than argued:
///
/// - **Its stated derivation was `m1.rs`'s and did not reach this page.** The alphas this
///   fixture produces are `{0, 29, 57, 86, 115, 172, 230}` — its quarter-pixel grid puts
///   an eighth of a cell under a 0.9-alpha rect at the corners — so ADR 0006's ±1 per
///   store amplified by `255/α` gives `⌈255/29⌉ = 9` here, not 2. The constant enforced
///   something nobody could re-derive from the sentence beside it, which is the same
///   defect ADR 0072 found in `m1.rs`.
/// - **No pixel of this page is stored to twice.** The 4 000 marks are 10.25 × 24.5 on a
///   14.75 × 32.5 pitch, so they do not overlap: the store histogram over the 2 005 644
///   pixels is `{0: 1 071 244, 1: 934 400}` exactly. Read at the pixel, the bound is
///   therefore `⌈255/α⌉` where there is ink — it takes the four values `{2, 3, 5, 9}`,
///   one per distinct non-zero alpha — and **0** where there is none.
/// - **The 0 is why this is worth doing on a clip page in particular**, and it is the one
///   place the new bound is *stronger* rather than merely honest. A clip that leaks admits
///   ink at pixels whose store count is zero — which is 1 071 244 of them, 53 % of the
///   raster — and that is exactly where a fixture-wide tolerance handed out slack.
///   Measured, with `rect_link_box` outset by 0.004 device pixels: **3 696 pixels are
///   inked where nothing stored, and `max_byte_diff` over the whole raster is 1**, so the
///   old gate passed. This one fails at (60, 79) — `got [0, 0, 0, 1], expected
///   [0, 0, 0, 0] — 1 unorm steps past a bound of 0 (0 stores at α 0)`.
///
/// Everywhere else the bound is *looser* than the constant it replaced — 9 at the α = 29
/// slivers against 2 — and that is the honest direction rather than a regression: 2 was
/// never derivable there, and ADR 0077 records the trade. The page and the reference in
/// fact agree **byte for byte** (largest raw difference over the whole raster: 0, llvmpipe,
/// 2026-08-23, as on 2026-08-17), so no slack of either shape is being spent today.
#[test]
fn three_hundred_three_identical_clips_collapse_to_one_region() {
    let mut device = device();
    let page = Rect::new(Point::new(60.0, 80.0), Point::new(1130.0, 1600.0));
    let clip_outline = device
        .upload_outline(&rect_outline(page))
        .expect("within budget");

    let mut builder = SceneBuilder::new();
    // 303 identifiers, one geometry — the caller's display list would deduplicate
    // these, but ours must collapse them by *region* even when nobody deduplicated.
    let clips: Vec<ClipId> = (0..303)
        .map(|_| {
            builder
                .clip(clip_outline, Affine::IDENTITY, FillRule::NonZero, None)
                .expect("valid clip")
        })
        .collect();
    // A dense page of small rects, each under one of the 303 clips; a band of them
    // crosses the clip edge so the clip visibly cuts coverage.
    for i in 0..4_000_usize {
        let x = (i % 80) as f32 * 14.75;
        let y = (i / 80) as f32 * 32.5;
        builder
            .rect(
                Rect::new(Point::new(x, y), Point::new(x + 10.25, y + 24.5)),
                Affine::IDENTITY,
                Color::new(0.1, 0.2, 0.3, 0.9),
                Some(clips[i % clips.len()]),
                None,
            )
            .expect("valid clipped rect");
    }
    let scene = builder.finish();
    assert_eq!(scene.cost().clips, 303);

    let viewport = Viewport::full(1191, 1684, Affine::IDENTITY);
    let frame = device
        .render(&scene, &viewport, Target::Readback)
        .expect("a full clipped page renders");
    assert_eq!(
        frame.counters().clip_distinct_regions,
        1,
        "303 identical clip states are one region — the region is the key, not the name"
    );

    let actual = frame.into_raster().unwrap().into_pixels();
    let expected = cpu_reference(&scene, &viewport, &vec![page; 303]);
    if let Some(where_) = disagreement(&actual, &expected, viewport.width) {
        panic!(
            "the clipped page differs from the reference beyond the store-conversion \
             bound (ADR 0006): {where_}"
        );
    }
}

/// An empty clip admits nothing, and that is different from an absent clip: same
/// command, three clip states, three answers — and the empty one is a legitimate
/// frame, not an error.
#[test]
fn empty_clip_admits_nothing_and_differs_from_absent() {
    let mut device = device();
    let onscreen = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(16.0, 16.0),
        )))
        .expect("upload");
    let offscreen = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(100.0, 100.0),
            Point::new(120.0, 120.0),
        )))
        .expect("upload");
    // A degenerate rectangle outline: recognised, and empty by construction.
    let degenerate = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(4.0, 0.0),
            Point::new(4.0, 16.0),
        )))
        .expect("upload");

    let fill = Rect::new(Point::new(2.0, 2.0), Point::new(14.0, 14.0));
    let color = Color::new(1.0, 0.0, 0.0, 1.0);
    let viewport = Viewport::full(16, 16, Affine::IDENTITY);

    let render_with = |device: &mut Device, clip_outline: Option<OutlineId>| {
        let mut builder = SceneBuilder::new();
        let clip = clip_outline.map(|outline| {
            builder
                .clip(outline, Affine::IDENTITY, FillRule::NonZero, None)
                .expect("valid clip")
        });
        builder
            .rect(fill, Affine::IDENTITY, color, clip, None)
            .expect("valid rect");
        device
            .render(&builder.finish(), &viewport, Target::Readback)
            .expect("all three clip states are legitimate frames")
            .into_raster()
            .unwrap()
            .into_pixels()
    };

    let absent = render_with(&mut device, None);
    let admitted = render_with(&mut device, Some(onscreen));
    let disjoint = render_with(&mut device, Some(offscreen));
    let empty = render_with(&mut device, Some(degenerate));

    // Unclipped and fully-admitting clip agree; both actually drew something.
    assert_eq!(absent, admitted);
    assert!(absent.iter().any(|&b| b != 0));
    // A chain that intersects to nothing draws nothing — same bytes as a blank frame.
    assert!(
        disjoint.iter().all(|&b| b == 0),
        "a disjoint clip admits nothing"
    );
    assert!(
        empty.iter().all(|&b| b == 0),
        "a degenerate clip admits nothing"
    );
}

/// A chain is an intersection: child ∩ parent, through the viewport transform,
/// including the y flip.
#[test]
fn chains_intersect_through_the_viewport() {
    let mut device = device();
    // In page space (y-up): parent admits the left half, child the bottom half.
    let parent_outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(8.0, 16.0),
        )))
        .expect("upload");
    let child_outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(16.0, 8.0),
        )))
        .expect("upload");

    let mut builder = SceneBuilder::new();
    let parent = builder
        .clip(parent_outline, Affine::IDENTITY, FillRule::NonZero, None)
        .expect("valid clip");
    let child = builder
        .clip(
            child_outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Some(parent),
        )
        .expect("valid chained clip");
    builder
        .rect(
            Rect::new(Point::new(0.0, 0.0), Point::new(16.0, 16.0)),
            Affine::IDENTITY,
            Color::new(0.0, 1.0, 0.0, 1.0),
            Some(child),
            None,
        )
        .expect("valid rect");
    let scene = builder.finish();

    // The y flip lives in the viewport (§3): page y-up, device y-down.
    let viewport = Viewport::full(
        16,
        16,
        Affine {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: -1.0,
            e: 0.0,
            f: 16.0,
        },
    );
    let raster = device
        .render(&scene, &viewport, Target::Readback)
        .expect("renders")
        .into_raster()
        .unwrap()
        .into_pixels();

    let pixel = |x: usize, y: usize| &raster[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4];
    // Page-space bottom-left quadrant = device rows 8.. — admitted.
    assert_eq!(pixel(3, 12), &[0, 255, 0, 255]);
    // Page-space top-left (device top) — cut by the child.
    assert_eq!(pixel(3, 3), &[0, 0, 0, 0]);
    // Page-space bottom-right — cut by the parent.
    assert_eq!(pixel(12, 12), &[0, 0, 0, 0]);
}

/// The caller's worst page: 3 608 chains — here one 3 608-link chain of shrinking
/// rectangles plus a command at every depth — resolves within the ordinary budgets,
/// with memoisation keeping it linear, and every distinct region counted.
#[test]
fn the_worst_page_of_chains_stays_within_budget() {
    let mut device = device();
    let mut builder = SceneBuilder::new();
    let mut parent: Option<ClipId> = None;
    let mut clips = Vec::with_capacity(3_608);
    for i in 0..3_608_u32 {
        // Each link shaves a sliver off the right edge: every chain resolves to a
        // distinct region.
        let outline = device
            .upload_outline(&rect_outline(Rect::new(
                Point::new(0.0, 0.0),
                Point::new(4_000.0 - i as f32, 4_000.0),
            )))
            .expect("within the resource budget");
        let clip = builder
            .clip(outline, Affine::IDENTITY, FillRule::NonZero, parent)
            .expect("valid chain link");
        clips.push(clip);
        parent = Some(clip);
    }
    for clip in &clips {
        builder
            .rect(
                Rect::new(Point::new(0.0, 0.0), Point::new(64.0, 64.0)),
                Affine::IDENTITY,
                Color::new(0.5, 0.5, 0.5, 0.01),
                Some(*clip),
                None,
            )
            .expect("valid rect");
    }
    let scene = builder.finish();
    assert_eq!(scene.cost().clips, 3_608);

    let frame = device
        .render(
            &scene,
            &Viewport::full(64, 64, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the worst page renders within the ordinary budgets");
    // Every referenced chain resolved; each link's region is distinct.
    assert_eq!(frame.counters().clip_distinct_regions, 3_608);
}

/// Refusals stay loud at the clip boundary: an outline the device has not got, and a
/// clip that is not a rectangle (its residue mask is M5's work).
#[test]
fn clip_refusals_are_named() {
    let mut device = device();
    let viewport = Viewport::full(8, 8, Affine::IDENTITY);
    let fill = Rect::new(Point::new(0.0, 0.0), Point::new(4.0, 4.0));
    let color = Color::new(0.0, 0.0, 0.0, 1.0);

    // A dangling outline id inside a clip definition.
    let mut dangling = SceneBuilder::new();
    let clip = dangling
        .clip(OutlineId(9_999), Affine::IDENTITY, FillRule::NonZero, None)
        .expect("the scene cannot know device residency");
    dangling
        .rect(fill, Affine::IDENTITY, color, Some(clip), None)
        .expect("valid rect");
    assert!(matches!(
        device.render(&dangling.finish(), &viewport, Target::Readback),
        Err(RenderError::UnknownOutline {
            outline: OutlineId(9_999)
        })
    ));

    // A triangular clip: not a rectangle, so it becomes M5's residue mask — and it
    // must actually mask. A full-target rect under it keeps the triangle's inside
    // and loses its outside.
    let triangle = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(8.0, 0.0)),
            Segment::LineTo(Point::new(4.0, 8.0)),
            Segment::Close,
        ])
        .expect("a triangle is a valid outline");
    let mut curved = SceneBuilder::new();
    let clip = curved
        .clip(triangle, Affine::IDENTITY, FillRule::NonZero, None)
        .expect("valid clip definition");
    curved
        .rect(
            Rect::new(Point::new(0.0, 0.0), Point::new(8.0, 8.0)),
            Affine::IDENTITY,
            Color::new(0.0, 0.0, 1.0, 1.0),
            Some(clip),
            None,
        )
        .expect("valid rect");
    let raster = device
        .render(&curved.finish(), &viewport, Target::Readback)
        .expect("non-rectangular clips mask since M5")
        .into_raster()
        .unwrap()
        .into_pixels();
    let pixel = |x: usize, y: usize| &raster[(y * 8 + x) * 4..(y * 8 + x) * 4 + 4];
    // Deep inside the triangle: fully admitted.
    assert_eq!(pixel(4, 1), &[0, 0, 255, 255]);
    // Outside the triangle (top corners): masked away entirely.
    assert_eq!(pixel(7, 6), &[0, 0, 0, 0]);
    assert_eq!(pixel(0, 6), &[0, 0, 0, 0]);
}

/// **Restating a clip changes nothing** — ISO 32000-2 §8.5.4 (ADR 0030).
///
/// > After the path has been painted, the clipping path in the graphics state shall be
/// > set to the intersection of the current clipping path and the newly constructed
/// > path.
///
/// A chain is one region arrived at by intersecting paths, so intersecting it with a
/// path it already contains is the same region — and a renderer that rasterises each
/// link separately owes that identity to the clause. Composing the links by multiplying
/// their coverages does not have it: an antialiased boundary at 0.5 raised to the *n*-th
/// power is the same clip stated *n* times and answered differently each time, which is
/// what the caller measured as a ladder halving at every rung
/// (`QUORRA_FEEDBACK.md` §18).
///
/// The diagonal is what makes the test bite: every pixel along it carries a fraction, and
/// only there do the two rules differ.
#[test]
fn a_clip_stated_again_admits_exactly_what_it_did() {
    let mut device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs");
    device.wait_until_warm();
    let side = 64_u32;
    let viewport = Viewport::full(side, side, Affine::IDENTITY);
    let diagonal = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(64.0, 61.0)),
            Segment::LineTo(Point::new(0.0, 64.0)),
            Segment::Close,
        ])
        .expect("a triangle is a valid outline");

    // The same clip, one to five times over, each time under the previous one.
    let mut under = |links: u32| {
        let mut builder = SceneBuilder::new();
        let mut clip = None;
        for _ in 0..links {
            clip = Some(
                builder
                    .clip(diagonal, Affine::IDENTITY, FillRule::NonZero, clip)
                    .expect("valid clip definition"),
            );
        }
        builder
            .rect(
                Rect::new(Point::new(0.0, 0.0), Point::new(64.0, 64.0)),
                Affine::IDENTITY,
                Color::new(0.0, 0.0, 0.0, 1.0),
                clip,
                None,
            )
            .expect("valid rect");
        device
            .render(&builder.finish(), &viewport, Target::Readback)
            .expect("a clipped rect draws")
            .into_raster()
            .unwrap()
            .into_pixels()
    };

    let once = under(1);
    for links in 2..=5 {
        let again = under(links);
        let worst = once
            .iter()
            .zip(&again)
            .map(|(a, b)| i32::from(*a).abs_diff(i32::from(*b)))
            .max()
            .unwrap_or(0);
        assert_eq!(
            worst, 0,
            "the same clip stated {links} times admits something else: {worst} of 255"
        );
    }
    // And the fixture has the fractional boundary the property is about: a diagonal
    // edge, so the chain is not trivially 0 or 255 everywhere it is composed.
    let edge = (0..side)
        .filter(|y| {
            let alpha = once[((y * side + 40) * 4 + 3) as usize];
            alpha > 0 && alpha < 255
        })
        .count();
    assert!(edge > 0, "the clip's boundary is antialiased somewhere");
}

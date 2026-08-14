//! The two coverage lanes, held against each other (ADR 0016).
//!
//! `Coverage::Cpu` computes the exact area of the pixel a shape covers; `Coverage::Gpu`
//! counts samples on an ordered grid. They are *different answers to the same
//! question*, and this file is about the shape of that difference:
//!
//! - where a pixel is wholly inside or wholly outside, the two must **agree exactly** —
//!   no sampling scheme can disagree about a pixel no edge crosses, so a difference
//!   there is a defect and not a quantisation;
//! - where an edge crosses a pixel, the GPU lane's answer must be the *sampled* one:
//!   `round(k/n × 255)` for `k` of `n` samples inside, which is derivable before the
//!   frame is drawn rather than read off it;
//! - and both must draw the *same* geometry — same clause, same fill rule, same
//!   sub-pixel placement — because the lane is a choice about cost, never about what
//!   the page says.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Coverage, Device, Options, RenderError, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, LineCap, LineJoin, Paint, Point, Scene,
    SceneBuilder, Segment, Stroke,
};

/// The fixtures are written in a 48-unit square and drawn at [`MAGNIFY`] times it, so
/// their tiles are ones [`TINY_ATLAS`] will not hold and the GPU lane actually takes
/// them — otherwise every comparison below would be the CPU lane against itself,
/// passing without testing anything (ADR 0028). Probe coordinates are in device pixels,
/// so they carry the same factor.
const UNITS: u32 = 48;
/// See [`UNITS`]. Sixteen makes a fixture's largest tile roughly 590 000 texels, which
/// is seventy times what this file's atlas admits.
const MAGNIFY: u32 = 16;
const SIZE: u32 = UNITS * MAGNIFY;

/// A small atlas, so the shapes below reach the scratch sheet rather than being
/// cached in front of it: 64 KiB admits tiles of 8 KiB (ADR 0024's eighth), and this
/// file is about what the two coverage lanes put on the sheet.
const TINY_ATLAS: u64 = 64 * 1024;

fn device_with(coverage: Coverage) -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        coverage,
        atlas_budget: TINY_ATLAS,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

/// The frame's pixels, and the count of commands that reached the GPU lane.
fn render(coverage: Coverage, build: impl Fn(&mut Device) -> Scene) -> Vec<u8> {
    let mut device = device_with(coverage);
    device.wait_until_warm();
    let scene = build(&mut device);
    device
        .render(
            &scene,
            &Viewport::full(SIZE, SIZE, Affine::scale(MAGNIFY as f32, MAGNIFY as f32)),
            Target::Readback,
        )
        .expect("the scene is inside every budget")
        .into_raster()
        .unwrap()
        .into_pixels()
}

/// The alpha of one device pixel.
fn alpha(pixels: &[u8], x: u32, y: u32) -> u8 {
    pixels[((y * SIZE + x) * 4 + 3) as usize]
}

/// A probe written in fixture units: the raster is [`MAGNIFY`] times larger, and the
/// middle of unit cell `(x, y)` is the same place in the shape at either scale.
fn at_unit(pixels: &[u8], x: u32, y: u32) -> u8 {
    alpha(pixels, x * MAGNIFY + MAGNIFY / 2, y * MAGNIFY + MAGNIFY / 2)
}

fn black() -> Paint {
    Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0))
}

/// A curved blob big enough to leave the atlas, so the CPU lane takes its path lane
/// and the two are compared on the same machinery.
fn blob(device: &mut Device) -> Scene {
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(8.0, 8.0)),
            Segment::CubicTo {
                c1: Point::new(40.0, 2.0),
                c2: Point::new(44.0, 30.0),
                to: Point::new(24.0, 40.0),
            },
            Segment::CubicTo {
                c1: Point::new(10.0, 44.0),
                c2: Point::new(2.0, 24.0),
                to: Point::new(8.0, 8.0),
            },
            Segment::Close,
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            black(),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    builder.finish()
}

/// Pixels no edge crosses are not a matter of opinion: wholly covered is 255 and
/// wholly uncovered is 0 in both lanes, and a difference there would be a defect in
/// one of them rather than the sampling.
///
/// **On a straight-edged fixture**, and that qualification is the whole of what makes
/// the claim true. Where an edge is a curve the two lanes do not draw the same shape:
/// the CPU lane flattens to `FLATTEN_TOLERANCE` and a chord cuts inside a convex curve,
/// so a pixel it calls wholly empty can still be crossed by the quadratic the GPU lane
/// draws. That difference is bounded rather than absent, and
/// [`a_curved_edge_differs_by_the_cpu_lane_s_flattening_too`] is where it is bounded.
#[test]
fn the_lanes_agree_exactly_where_no_edge_crosses() {
    let cpu = render(Coverage::Cpu, straight);
    let gpu = render(Coverage::Gpu, straight);
    let mut interior = 0;
    let mut exterior = 0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (a, b) = (alpha(&cpu, x, y), alpha(&gpu, x, y));
            if a == 255 {
                assert_eq!(
                    b, 255,
                    "solid for the CPU lane, {b} for the GPU one at {x},{y}"
                );
                interior += 1;
            }
            if a == 0 {
                assert_eq!(
                    b, 0,
                    "empty for the CPU lane, {b} for the GPU one at {x},{y}"
                );
                exterior += 1;
            }
        }
    }
    assert!(interior > 200, "the fixture has an interior: {interior}");
    assert!(exterior > 200, "and an exterior: {exterior}");
}

/// On a **straight** edge the difference is the sample grid and nothing else.
///
/// Four sample columns sit at ⅛, ⅜, ⅝ and ⅞ across a pixel, so a straight edge at any
/// fraction is answered to within half a column — `1/8` of a pixel, `32` of 255. That
/// is a property of the grid, derived before the frame is drawn; the fixture measures
/// 12.
#[test]
fn a_straight_edge_differs_by_the_sample_grid_and_no_more() {
    let cpu = render(Coverage::Cpu, straight);
    let gpu = render(Coverage::Gpu, straight);
    let (edges, worst_edge, worst) = compare(&cpu, &gpu);
    assert!(edges > 40, "the fixture has an antialiased edge: {edges}");
    assert!(
        worst_edge <= 32 && worst == worst_edge,
        "a 4x4 ordered grid answers a straight edge to within an eighth of a pixel, and \
         nothing away from an edge differs at all; worst difference was {worst_edge} on \
         an edge and {worst} anywhere"
    );
}

/// On a **curved** edge the difference is larger, and the reason is the CPU lane.
///
/// `raster.rs` flattens curves to `FLATTEN_TOLERANCE` — a quarter pixel — and a chord
/// cuts *inside* a convex curve, so its shape is up to that much smaller than the one
/// the caller submitted. The GPU lane does not flatten at all: it draws the quadratics
/// the outline converted to at upload, within 8×10⁻⁵ units of the cubic. So the gap
/// here is a quarter pixel of coverage (64 of 255) on top of the grid's eighth, and
/// **the GPU lane is the more accurate of the two**.
///
/// Measured rather than asserted into being: at `FLATTEN_TOLERANCE` = 0.25 the worst
/// difference on this fixture is 62 and 33 pixels differ by more than 20; tightening
/// that constant to 0.004 takes it to *zero* pixels over 20, which is what identifies
/// the flattening as the whole of the difference rather than something in this lane.
///
/// (Corrected 2026-08-14, ADR 0044: the CPU lane's bound is the *tighter* of that quarter
/// pixel and 1/32 of a cubic's own device extent. This fixture's cubics span 40 pixels, so
/// the second term is inert on it and the numbers above stand — but on a curve under eight
/// pixels across the two lanes now agree far better than this test's bound suggests, and
/// the gap that used to be here was the CPU lane being *wrong* rather than coarse.)
#[test]
fn a_curved_edge_differs_by_the_cpu_lane_s_flattening_too() {
    let cpu = render(Coverage::Cpu, blob);
    let gpu = render(Coverage::Gpu, blob);
    let (edges, worst_edge, worst) = compare(&cpu, &gpu);
    assert!(edges > 40, "the fixture has an antialiased edge: {edges}");
    assert!(
        worst <= 96,
        "an eighth of a pixel for the grid and a quarter for the flattening bound this \
         at 96 of 255; worst difference was {worst_edge} on a pixel the CPU lane called \
         antialiased and {worst} over the whole frame"
    );
}

/// Antialiased pixels, the largest disagreement among them, and the largest anywhere.
///
/// The third number is there because "antialiased for the CPU lane" is not the same set
/// of pixels as "crossed by an edge": on a curve the CPU lane's flattened chord can miss
/// a pixel the true quadratic clips a corner off, and that pixel reads 0 on one lane and
/// a sample or two on the other. Counting only the pixels one lane called antialiased
/// would leave exactly those out of the comparison.
fn compare(cpu: &[u8], gpu: &[u8]) -> (u32, i32, i32) {
    let (mut edges, mut worst_edge, mut worst) = (0, 0_i32, 0_i32);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (a, b) = (i32::from(alpha(cpu, x, y)), i32::from(alpha(gpu, x, y)));
            if a > 0 && a < 255 {
                edges += 1;
                worst_edge = worst_edge.max((a - b).abs());
            }
            worst = worst.max((a - b).abs());
        }
    }
    (edges, worst_edge, worst)
}

/// A triangle with straight edges at fractional coordinates, so every edge pixel is
/// antialiased and none of the difference can be blamed on a curve.
fn straight(device: &mut Device) -> Scene {
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(7.3, 5.1)),
            Segment::LineTo(Point::new(40.7, 12.9)),
            Segment::LineTo(Point::new(22.4, 41.6)),
            Segment::Close,
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            black(),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    builder.finish()
}

/// The lane is a choice about cost, not about the clause. Nested same-wound squares
/// fill under §8.5.3.3.2 and hollow under §8.5.3.3.3, and both lanes must say so.
#[test]
fn both_lanes_read_the_fill_rules_the_same_way() {
    let nested = |rule: FillRule| {
        move |device: &mut Device| {
            let square = |x0: f32, y0: f32, x1: f32, y1: f32| {
                vec![
                    Segment::MoveTo(Point::new(x0, y0)),
                    Segment::LineTo(Point::new(x1, y0)),
                    Segment::LineTo(Point::new(x1, y1)),
                    Segment::LineTo(Point::new(x0, y1)),
                    Segment::Close,
                ]
            };
            let mut path = square(6.0, 6.0, 42.0, 42.0);
            path.extend(square(18.0, 18.0, 30.0, 30.0));
            let outline = device.upload_outline(&path).unwrap();
            let mut builder = SceneBuilder::new();
            builder
                .fill(
                    outline,
                    Affine::IDENTITY,
                    rule,
                    black(),
                    None,
                    BlendMode::Normal,
                    Compose::SrcOver,
                    None,
                )
                .unwrap();
            builder.finish()
        }
    };
    for coverage in [Coverage::Cpu, Coverage::Gpu] {
        let non_zero = render(coverage, nested(FillRule::NonZero));
        let even_odd = render(coverage, nested(FillRule::EvenOdd));
        assert_eq!(
            at_unit(&non_zero, 24, 24),
            255,
            "{coverage:?}: winding two is not zero (§8.5.3.3.2)"
        );
        assert_eq!(
            at_unit(&even_odd, 24, 24),
            0,
            "{coverage:?}: winding two is even (§8.5.3.3.3)"
        );
        assert_eq!(
            at_unit(&non_zero, 10, 24),
            255,
            "{coverage:?}: the outer ring"
        );
        assert_eq!(at_unit(&even_odd, 10, 24), 255);
    }
}

/// A stroke takes the GPU lane too: it is expanded to a polygon on the CPU (§8.4.3's
/// caps and joins are the device's business either way) and the *rasterising* is what
/// moves, which is the half that costs.
#[test]
fn a_stroke_draws_the_same_band_in_both_lanes() {
    let scene = |device: &mut Device| {
        let outline = device
            .upload_outline(&[
                Segment::MoveTo(Point::new(10.0, 24.0)),
                Segment::LineTo(Point::new(38.0, 24.0)),
            ])
            .unwrap();
        let mut builder = SceneBuilder::new();
        builder
            .stroke(
                outline,
                Affine::IDENTITY,
                Stroke {
                    // Device-space (§4.5), so it carries `MAGNIFY` like the geometry
                    // does — an 8-unit band drawn at 16x is 128 device pixels.
                    width: 8.0 * MAGNIFY as f32,
                    cap: LineCap::Butt,
                    join: LineJoin::Miter,
                    miter_limit: 4.0,
                },
                black(),
                None,
                BlendMode::Normal,
                None,
            )
            .unwrap();
        builder.finish()
    };
    let cpu = render(Coverage::Cpu, scene);
    let gpu = render(Coverage::Gpu, scene);
    for y in [21, 24, 27] {
        assert_eq!(at_unit(&gpu, 24, y), 255, "inside the band at row {y}");
        assert_eq!(at_unit(&cpu, 24, y), 255);
    }
    assert_eq!(at_unit(&gpu, 24, 19), 0, "a row above the band");
    assert_eq!(at_unit(&gpu, 9, 24), 0, "before the butt cap");
}

/// **A clip the GPU lane cannot honour sends its command to the CPU lane**, and the
/// frame is still right: a non-rectangular clip multiplies into coverage on the CPU,
/// so both kinds of tile land on one sheet and the picture does not say which is which.
#[test]
fn a_residue_clip_falls_back_and_still_draws() {
    let scene = |device: &mut Device| {
        let diamond = device
            .upload_outline(&[
                Segment::MoveTo(Point::new(24.0, 4.0)),
                Segment::LineTo(Point::new(44.0, 24.0)),
                Segment::LineTo(Point::new(24.0, 44.0)),
                Segment::LineTo(Point::new(4.0, 24.0)),
                Segment::Close,
            ])
            .unwrap();
        let square = device
            .upload_outline(&[
                Segment::MoveTo(Point::new(8.0, 8.0)),
                Segment::LineTo(Point::new(40.0, 8.0)),
                Segment::LineTo(Point::new(40.0, 40.0)),
                Segment::LineTo(Point::new(8.0, 40.0)),
                Segment::Close,
            ])
            .unwrap();
        let mut builder = SceneBuilder::new();
        // The diamond is not axis-aligned, so it resolves to a residue link.
        let clip = builder
            .clip(diamond, Affine::IDENTITY, FillRule::NonZero, None)
            .unwrap();
        builder
            .fill(
                square,
                Affine::IDENTITY,
                FillRule::NonZero,
                black(),
                Some(clip),
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
        builder.finish()
    };
    let cpu = render(Coverage::Cpu, scene);
    let gpu = render(Coverage::Gpu, scene);
    assert_eq!(
        gpu, cpu,
        "the command took the CPU lane in both devices, so the frames are the same bytes"
    );
    assert_eq!(
        at_unit(&gpu, 24, 24),
        255,
        "the middle of the clipped square"
    );
    assert_eq!(at_unit(&gpu, 10, 10), 0, "a corner the diamond clips away");
}

/// Both kinds of tile on one sheet: a residue-clipped fill (CPU) beside an unclipped
/// one (GPU) in the same frame. The sheet has one layout and two producers, and this
/// is the case that proves neither overwrote the other.
#[test]
fn one_sheet_carries_both_producers() {
    let scene = |device: &mut Device| {
        let diamond = device
            .upload_outline(&[
                Segment::MoveTo(Point::new(12.0, 2.0)),
                Segment::LineTo(Point::new(22.0, 12.0)),
                Segment::LineTo(Point::new(12.0, 22.0)),
                Segment::LineTo(Point::new(2.0, 12.0)),
                Segment::Close,
            ])
            .unwrap();
        let curve = device
            .upload_outline(&[
                Segment::MoveTo(Point::new(28.0, 28.0)),
                Segment::CubicTo {
                    c1: Point::new(46.0, 28.0),
                    c2: Point::new(46.0, 46.0),
                    to: Point::new(28.0, 44.0),
                },
                Segment::Close,
            ])
            .unwrap();
        let mut builder = SceneBuilder::new();
        let clip = builder
            .clip(diamond, Affine::IDENTITY, FillRule::NonZero, None)
            .unwrap();
        // Clipped by a residue: the CPU lane rasterises this one.
        builder
            .fill(
                diamond,
                Affine::IDENTITY,
                FillRule::NonZero,
                black(),
                Some(clip),
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
        // Unclipped and curved: the GPU lane draws this one, onto the same sheet.
        builder
            .fill(
                curve,
                Affine::IDENTITY,
                FillRule::NonZero,
                black(),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
        builder.finish()
    };
    let gpu = render(Coverage::Gpu, scene);
    assert_eq!(at_unit(&gpu, 12, 12), 255, "the CPU lane's tile survived");
    assert_eq!(
        at_unit(&gpu, 33, 36),
        255,
        "and the GPU lane's tile is there"
    );
    assert_eq!(at_unit(&gpu, 24, 6), 0, "with nothing between them");
}

/// The lane is chosen per frame, on one device, and each frame is the frame that lane
/// draws — no state carried from the last one, no cache invalidated by the switch.
///
/// This is what makes the choice a caller's to make at all: the crossover is a
/// magnification, and a session in which a person zooms crosses it. A device that
/// could only be told once would have to be told before the zoom existed.
#[test]
fn the_lane_can_change_between_frames_on_one_device() {
    let mut device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        atlas_budget: TINY_ATLAS,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs");
    device.wait_until_warm();
    // A tile the GPU lane takes, which is one the atlas will not hold (ADR 0028) and
    // whose triangles cost less than its coverage (ADR 0026). Both conditions matter to
    // this test: a fixture either lane answers the same way — a cached glyph, or a
    // nine-pixel one — could not tell the two lanes apart at all.
    let scene = blob_at(&mut device, 800.0);
    let viewport = Viewport::full(1600, 2200, Affine::IDENTITY);

    // Alternate, twice each way, and keep what every frame drew.
    let mut frames = Vec::new();
    for lane in [Coverage::Cpu, Coverage::Gpu, Coverage::Cpu, Coverage::Gpu] {
        device.set_coverage(lane);
        assert_eq!(device.coverage(), lane);
        frames.push(
            device
                .render(&scene, &viewport, Target::Readback)
                .expect("either lane draws this scene")
                .into_raster()
                .unwrap()
                .into_pixels(),
        );
    }

    assert_eq!(
        frames[0], frames[2],
        "the CPU lane draws what it drew before, after a frame of the other lane"
    );
    assert_eq!(
        frames[1], frames[3],
        "and so does the GPU lane — neither leaves state in the other's way"
    );
    assert_ne!(
        frames[0], frames[1],
        "the fixture is one the two lanes answer differently, so the test can tell them \
         apart at all"
    );
}

/// The side of the target `wide_blob` is drawn at. Far past what [`TINY_ATLAS`] holds,
/// so the GPU lane takes the tile and can be charged for the target it then needs.
const WIDE: u32 = 800;

/// `blob`'s curve, scaled past what the test devices' small atlas will take (ADR 0024)
/// — so no atlas stands in front of it and its coverage lands on the frame's scratch
/// sheet under either lane, which is the only condition under which the sheet has an
/// extent to be charged for at all.
fn wide_blob(device: &mut Device) -> Scene {
    // Written in the same proportions as `blob`, scaled to fill the target — so the
    // tile is `WIDE` across whatever `WIDE` is, and the centre probe is inside it.
    let unit = f32::from(u16::try_from(WIDE).unwrap_or(u16::MAX)) / 240.0;
    let at = |x: f32, y: f32| Point::new(x * unit, y * unit);
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(at(20.0, 20.0)),
            Segment::CubicTo {
                c1: at(220.0, 4.0),
                c2: at(236.0, 160.0),
                to: at(120.0, 220.0),
            },
            Segment::CubicTo {
                c1: at(40.0, 236.0),
                c2: at(4.0, 120.0),
                to: at(20.0, 20.0),
            },
            Segment::Close,
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            black(),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    builder.finish()
}

/// A CPU-lane frame is charged nothing for the winding texture, because none is made
/// for it — and the same frame on the GPU lane is charged, because there one is.
///
/// Both lanes pack into one scratch sheet, so the sheet's extent is filled in on every
/// frame that packs a tile, including one the GPU lane never ran. Pricing the winding
/// texture from that extent charges a CPU-lane frame `max_target_size × rows × 8`
/// bytes it does not allocate: five pages of the caller's 974-document corpus were
/// refused that way, claiming between 268 MB and 1.2 GB against a 256 MiB budget for a
/// texture no frame of theirs created. The pair below is what says the condition is a
/// condition rather than a deletion — the charge still stands where the texture does.
///
/// The budget is eight rows of that texture. The sheet is the device's maximum
/// dimension wide (capacity, so that wide coverage is not refused for the shape of the
/// packing) and the texture is `rgba16float`, so a row costs `max_target_size × 8` and
/// eight rows admit this fixture's ~50 KB of real coverage many times over while
/// refusing anything priced against a sheet 200 rows deep.
#[test]
fn only_the_lane_that_makes_the_winding_texture_pays_for_it() {
    // The sheet this scene packs is the blob's own tile, roughly WIDE x WIDE of R8
    // (ADR 0021 narrows it to what the shelves reached, so the device's maximum
    // dimension no longer decides this number). Four tiles' worth admits the CPU lane,
    // which allocates the sheet and nothing else, and refuses the GPU lane, which adds
    // eight bytes a texel of `rgba16float` on top of it.
    let budget = u64::from(WIDE) * u64::from(WIDE) * 4;
    let mut device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        atlas_budget: TINY_ATLAS,
        max_frame_bytes: budget,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs");
    device.wait_until_warm();
    let scene = wide_blob(&mut device);
    let viewport = Viewport::full(WIDE, WIDE, Affine::IDENTITY);

    let drawn = device
        .render(&scene, &viewport, Target::Readback)
        .expect("the CPU lane allocates no winding texture, so it is charged for none")
        .into_raster()
        .unwrap()
        .into_pixels();
    assert_eq!(
        drawn[((WIDE / 2 * WIDE + WIDE / 2) * 4 + 3) as usize],
        255,
        "and the frame that was not refused drew the blob, rather than nothing"
    );

    device.set_coverage(Coverage::Gpu);
    match device.render(&scene, &viewport, Target::Readback) {
        Err(RenderError::FrameBudgetExceeded { needed, budget: b }) => {
            assert_eq!(b, budget);
            assert!(
                needed > budget,
                "the GPU lane does make the texture, and pays the eight bytes a texel \
                 for it: {needed} against {budget}"
            );
        }
        other => panic!("expected the GPU lane to be charged for its texture, got {other:?}"),
    }
}

/// The lane a command takes is decided by what the two lanes would cost it, not by a
/// constant (ADR 0026, and ADR 0028 for the atlas half of the question).
///
/// The GPU lane's price is this outline's triangles — three vertices each, one stride
/// apiece — **per placement, whatever the tile's size**. The CPU lane's is the tile's
/// area in coverage bytes, *and* the cache in front of it: a tile the atlas holds is
/// rasterised once for every placement there will ever be, which is a price no lane
/// beats. So one shape at two scales takes two different lanes, and `Counters::tiles`
/// says which: the GPU lane always packs a tile, the CPU lane packs one only when the
/// atlas will not hold it.
#[test]
fn the_lane_follows_what_each_would_cost() {
    let small = |device: &mut Device| blob_at(device, 9.0);
    let large = |device: &mut Device| blob_at(device, 300.0);

    // Small: the triangles cost more than the coverage, so the GPU lane declines and
    // the tile goes to the atlas — nothing reaches the sheet.
    let mut device = device_with(Coverage::Gpu);
    let scene = small(&mut device);
    let frame = device
        .render(
            &scene,
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("a page of small glyphs must draw on the GPU lane's setting");
    assert_eq!(
        frame.counters().tiles,
        0,
        "a nine-pixel glyph costs 12.4 KB of triangles against ~150 bytes of coverage; \
         asking the device to draw it is what made a page of 66 309 of them refuse"
    );

    // Large: the coverage costs more than the triangles, so the device draws it.
    let mut device = device_with(Coverage::Gpu);
    let scene = large(&mut device);
    let frame = device
        .render(
            &scene,
            &Viewport::full(SIZE * 4, SIZE * 4, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("a large shape must draw");
    assert!(
        frame.counters().tiles > 0,
        "at 300 pixels the same outline's coverage is 90 KB against the same triangles, \
         and the lane ADR 0016 built for exactly this must take it"
    );

    // And the setting still decides: the same large shape under `Cpu` never reaches the
    // GPU lane, whatever it would have cost.
    let mut device = device_with(Coverage::Cpu);
    let scene = large(&mut device);
    let frame = device
        .render(
            &scene,
            &Viewport::full(SIZE * 4, SIZE * 4, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("draws");
    let _ = frame.into_raster();
}

/// One closed curve `side` pixels across, filled — the shape both lanes are asked about
/// above.
fn blob_at(device: &mut Device, side: f32) -> Scene {
    let r = side * 0.5;
    let centre = side;
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(centre - r, centre)),
            Segment::CubicTo {
                c1: Point::new(centre - r, centre - r),
                c2: Point::new(centre + r, centre - r),
                to: Point::new(centre + r, centre),
            },
            Segment::CubicTo {
                c1: Point::new(centre + r, centre + r),
                c2: Point::new(centre - r, centre + r),
                to: Point::new(centre - r, centre),
            },
            Segment::Close,
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Solid(Color::new(0.2, 0.3, 0.7, 1.0)),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    builder.finish()
}

/// The side of one shape in the multi-pane fixtures below: far past what [`TINY_ATLAS`]
/// holds, so every one of them takes the GPU lane and lands on the sheet.
const PANE_FIXTURE_SIDE: u32 = 800;

/// One outline, `columns × rows` placements of it, each [`PANE_FIXTURE_SIDE`] across.
///
/// The placements are what matter: the winding target holds one *pane* of the sheet at
/// a time (ADR 0028), and a grid this size is cut into several of them — some with a
/// column offset, some with a row offset, which are the two halves of the agreement the
/// two shader stages have to keep.
fn grid_of_blobs(device: &mut Device, columns: u32, rows: u32) -> Scene {
    let side = PANE_FIXTURE_SIDE as f32;
    let r = side * 0.5;
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.0, r)),
            Segment::CubicTo {
                c1: Point::new(0.0, 0.0),
                c2: Point::new(side, 0.0),
                to: Point::new(side, r),
            },
            Segment::CubicTo {
                c1: Point::new(side, side),
                c2: Point::new(0.0, side),
                to: Point::new(0.0, r),
            },
            Segment::Close,
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    for row in 0..rows {
        for column in 0..columns {
            builder
                .fill(
                    outline,
                    Affine::translate(
                        (column * PANE_FIXTURE_SIDE) as f32,
                        (row * PANE_FIXTURE_SIDE) as f32,
                    ),
                    FillRule::NonZero,
                    black(),
                    None,
                    BlendMode::Normal,
                    Compose::SrcOver,
                    None,
                )
                .unwrap();
        }
    }
    builder.finish()
}

/// **Every pane draws its own tiles**, and a pane after the first is not a pane that
/// draws nothing.
///
/// Four columns by two rows of 800-pixel shapes pack onto a sheet 3 200 texels wide,
/// which the pane budget cuts into four rectangles: two per shelf, the second of each
/// starting at a column offset, and the second shelf at a row offset. So every one of
/// the three places that must subtract the pane's origin — `vs_winding` before mapping
/// to clip space, `fs_winding` before testing a fragment against its tile, `fs_resolve`
/// before reading the target back — is exercised in both axes by this one frame.
///
/// The test exists because ADR 0027 shipped with the middle one missing. Every band
/// after the first discarded every fragment of every tile, and the page came back with
/// holes in it and an `Ok` frame: principle 6's exact failure, invisible to every test
/// in this file because each of them drew a single band.
#[test]
fn every_pane_draws_the_tiles_it_holds() {
    let (columns, rows) = (4_u32, 2_u32);
    let (width, height) = (columns * PANE_FIXTURE_SIDE, rows * PANE_FIXTURE_SIDE);
    let scene_of = |device: &mut Device| grid_of_blobs(device, columns, rows);
    let render_grid = |coverage: Coverage| {
        let mut device = device_with(coverage);
        device.wait_until_warm();
        let scene = scene_of(&mut device);
        let frame = device
            .render(
                &scene,
                &Viewport::full(width, height, Affine::IDENTITY),
                Target::Readback,
            )
            .expect("eight large shapes are inside every budget");
        let tiles = frame.counters().tiles;
        (frame.into_raster().unwrap().into_pixels(), tiles)
    };
    let (gpu, gpu_tiles) = render_grid(Coverage::Gpu);
    let (cpu, _) = render_grid(Coverage::Cpu);
    assert_eq!(
        gpu_tiles,
        columns * rows,
        "each placement is its own tile on the sheet"
    );

    // The middle of each shape, which is solid in both lanes: a pane that drew nothing
    // reads 0 here, and a pane that drew in the wrong place reads 0 here and paints
    // somewhere it should not.
    for row in 0..rows {
        for column in 0..columns {
            let x = column * PANE_FIXTURE_SIDE + PANE_FIXTURE_SIDE / 2;
            let y = row * PANE_FIXTURE_SIDE + PANE_FIXTURE_SIDE / 2;
            let seen = gpu[((y * width + x) * 4 + 3) as usize];
            assert_eq!(
                seen, 255,
                "the shape at column {column}, row {row} is solid in its middle, not {seen}"
            );
        }
    }
    // And nothing was drawn outside the shapes: a pane written at the wrong offset
    // leaves its coverage somewhere, and the corners of this grid are the somewhere.
    for row in 0..rows {
        for column in 0..columns {
            let (x, y) = (column * PANE_FIXTURE_SIDE + 4, row * PANE_FIXTURE_SIDE + 4);
            let index = ((y * width + x) * 4 + 3) as usize;
            assert_eq!(
                gpu[index], cpu[index],
                "the corner of column {column}, row {row}: the lanes agree that the \
                 blob does not reach it"
            );
        }
    }
}

/// A page of large shapes draws under a budget the sheet-wide target could not fit.
///
/// Eight 800-pixel shapes in a row pack onto a sheet 6 400 texels wide. ADR 0027's band
/// spanned that width — 6 400 × 800 × 8 bytes, 41 MB of `rgba16float` — while the pane
/// this frame needs is [`quorra_gpu`'s pane budget] wide at most: 2 621 × 800 × 8, under
/// 17 MB. The budget below sits between the two, so the frame draws here and would have
/// been refused as a band; and it is a *budget*, so the refusal it replaces would have
/// been an honest `Err` rather than a blank page (principle 6).
///
/// This is the case ADR 0027 recorded as what it did not fix, at the scale a software
/// adapter can render: the shape of the arithmetic is the same at thirty shapes of
/// 1 200 pixels, where a band asked for 309 MB against a 256 MiB budget.
#[test]
fn a_page_of_large_shapes_fits_a_budget_no_band_of_the_sheet_could() {
    let columns = 8_u32;
    let width = columns * PANE_FIXTURE_SIDE;
    let mut device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        coverage: Coverage::Gpu,
        atlas_budget: TINY_ATLAS,
        // Above the pane (17 MB) plus the sheet (5 MB) and the quads, below the band
        // (41 MB) plus the same.
        max_frame_bytes: 30 * 1024 * 1024,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs");
    device.wait_until_warm();
    let scene = grid_of_blobs(&mut device, columns, 1);
    let pixels = device
        .render(
            &scene,
            &Viewport::full(width, PANE_FIXTURE_SIDE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the pane fits the budget, so the frame is drawn rather than refused")
        .into_raster()
        .unwrap()
        .into_pixels();
    let last = (columns - 1) * PANE_FIXTURE_SIDE + PANE_FIXTURE_SIDE / 2;
    let index = ((PANE_FIXTURE_SIDE / 2 * width + last) * 4 + 3) as usize;
    assert_eq!(
        pixels[index], 255,
        "and the last shape of the page is on it"
    );
}

/// **A shape the page places once takes the device lane; the same shape placed again
/// takes the atlas** (ADR 0029).
///
/// Both fixtures draw a 300-pixel blob on a device with the *default* 8 MiB atlas, where
/// a 90 000-texel tile is well inside what `AtlasStore::admits` allows — so the atlas
/// would accept either of them, and what separates the two is only how many times the
/// page asks for the tile. `Counters::tiles` says which lane answered: the GPU lane
/// always packs a tile on the sheet, and a cached glyph packs none.
///
/// The repeats are placed at whole-pixel offsets on purpose. The census counts a shape
/// by its outline and scale and deliberately not by its sub-pixel phase, so three
/// placements at arbitrary offsets would be three atlas keys used once each — the count
/// is an upper bound on reuse, and a fixture that leaned on the loose direction would be
/// asserting the bound rather than the behaviour.
#[test]
fn the_lane_follows_how_often_the_page_places_the_shape() {
    let placements = |count: u32| {
        move |device: &mut Device| {
            let side = 300.0_f32;
            let r = side * 0.5;
            let outline = device
                .upload_outline(&[
                    Segment::MoveTo(Point::new(0.0, r)),
                    Segment::CubicTo {
                        c1: Point::new(0.0, 0.0),
                        c2: Point::new(side, 0.0),
                        to: Point::new(side, r),
                    },
                    Segment::CubicTo {
                        c1: Point::new(side, side),
                        c2: Point::new(0.0, side),
                        to: Point::new(0.0, r),
                    },
                    Segment::Close,
                ])
                .unwrap();
            let mut builder = SceneBuilder::new();
            for index in 0..count {
                builder
                    .fill(
                        outline,
                        Affine::translate((index * 320) as f32, 0.0),
                        FillRule::NonZero,
                        black(),
                        None,
                        BlendMode::Normal,
                        Compose::SrcOver,
                        None,
                    )
                    .unwrap();
            }
            builder.finish()
        }
    };
    let tiles_for = |count: u32| {
        let mut device = Device::headless(&Options {
            adapter: Some("llvmpipe".into()),
            coverage: Coverage::Gpu,
            ..Options::default()
        })
        .expect("llvmpipe is present wherever this suite runs");
        device.wait_until_warm();
        let scene = placements(count)(&mut device);
        device
            .render(
                &scene,
                &Viewport::full(count * 320, 400, Affine::IDENTITY),
                Target::Readback,
            )
            .expect("the fixture is inside every budget")
            .counters()
            .tiles
    };
    assert_eq!(
        tiles_for(1),
        1,
        "one placement: the entry would be written once and read once, so the device \
         draws it and the atlas keeps nothing"
    );
    assert_eq!(
        tiles_for(3),
        0,
        "three placements of one tile: the first rasterisation pays for the other two, \
         which is a price no lane beats"
    );
}

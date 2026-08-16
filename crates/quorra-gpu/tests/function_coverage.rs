//! A §7.10.5 program painted on a device set to [`Coverage::Gpu`] (ADR 0053, ADR 0016).
//!
//! # Why this file exists
//!
//! Every other function test runs the default [`Coverage::Cpu`]
//! (`doc/notes-function-wiring.md` §4.5), so one of the two settings a caller can put a
//! device in had never drawn this paint at all.
//!
//! # What the two settings are allowed to differ by here, and why it is not ADR 0016's
//! bound
//!
//! `tests/coverage_lanes.rs` bounds the *lanes* at an eighth of a pixel for the sample
//! grid plus a quarter for the CPU lane's flattening, and ADR 0006 bounds two *adapters*
//! at ±1 unorm. **Neither is the bound between the two settings for this paint, and the
//! reason is a property of the encoder rather than a measurement**: `Encoder::take_gpu_lane`
//! is consulted in exactly two places, `encode/fill.rs`'s `fill_solid` and
//! `encode/coverage.rs`'s `push_coverage_styled`, and both are the **solid** arm. A rare
//! paint reaches the sheet through `encode/rare.rs`'s `push_rare_coverage`, which calls
//! `coverage_tile` directly, so a function fill's coverage is the CPU rasteriser's under
//! either setting and the two frames are the **same bytes**.
//!
//! That is a stronger claim than a bound, so it is the one asserted — and it is asserted
//! rather than reasoned about, because it is exactly the kind of statement that stops being
//! true the day the rare lanes learn the device path and nothing notices.
//!
//! The interaction the setting *does* create is that the frame's scratch sheet then has two
//! producers, and a function quad's texel arithmetic (`coverage.xy + p − dest.xy`) assumes
//! its own tile is where the packer put it. `a_function_tile_and_a_device_drawn_tile_share_one_sheet`
//! is that case.

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

use quorra_gpu::{Coverage, Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, FnOp, FnRange, FunctionId, OutlineId, Paint,
    Point, Rect, Scene, SceneBuilder, Segment,
};

/// The target the pages below are drawn into. 192 × 4 = 768 bytes a row, a multiple of
/// the 256-byte buffer-copy alignment, and wide enough that the two marks of
/// [`a_function_tile_and_a_device_drawn_tile_share_one_sheet`] do not overlap — which they
/// have to not do, because each half of that page names one lane.
const SIZE: u32 = 192;

/// A small atlas, as in `tests/coverage_lanes.rs`: 64 KiB admits tiles of 8 KiB
/// (ADR 0024's eighth), so the solid mark below is refused by the cache and the GPU lane
/// is a live option for it. Without that, `take_gpu_lane`'s fourth condition declines and
/// the frame would compare one lane with itself — the trap `doc/HANDOVER.md` records from
/// `m45.rs`.
const TINY_ATLAS: u64 = 64 * 1024;

/// `x y 0.5` — the colour at a point is its own position, so a displaced quad cannot draw
/// the right picture by accident.
const POSITION_IS_COLOUR: [FnOp; 1] = [FnOp::PushReal(0.5)];

/// The device this file's assertions are made on, named so a failure says where
/// (ADR 0053 promises no cross-adapter identity for this paint). `QUORRA_ADAPTER` picks
/// another; the suite's default is the software rasteriser.
fn device(coverage: Coverage) -> (Device, String) {
    let requested = std::env::var("QUORRA_ADAPTER").unwrap_or_else(|_| "llvmpipe".into());
    let device = Device::headless(&Options {
        adapter: Some(requested),
        coverage,
        atlas_budget: TINY_ATLAS,
        ..Options::default()
    })
    .expect("the requested adapter is present");
    let name = device.description().to_string();
    (device, name)
}

fn rect_outline(rect: Rect) -> Vec<Segment> {
    vec![
        Segment::MoveTo(rect.min),
        Segment::LineTo(Point::new(rect.max.x, rect.min.y)),
        Segment::LineTo(rect.max),
        Segment::LineTo(Point::new(rect.min.x, rect.max.y)),
        Segment::Close,
    ]
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * SIZE + x) * 4) as usize;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

/// One function-painted fill whose `Matrix` maps the unit square onto a `side`-pixel square
/// at `origin`, so the value at a pixel is its position within that square (§10.7.4 for the
/// centre).
fn function_fill(
    builder: &mut SceneBuilder,
    outline: OutlineId,
    program: FunctionId,
    origin: (f32, f32),
    side: f32,
) {
    let place = Affine::translate(origin.0, origin.1);
    builder
        .fill(
            outline,
            place,
            FillRule::NonZero,
            Paint::Function {
                program,
                domain: Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
                matrix: Affine::scale(side, side).then(place),
                range: FnRange::Rgb([[0.0, 1.0]; 3]),
                background: None,
            },
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("a valid function fill");
}

/// A page drawing one program through **both** of the lane's quad placements: a rectangle,
/// which takes the analytic rect-hinted path, and a triangle, which takes a rasterised
/// coverage tile on the frame's sheet.
///
/// Both are on the page because only the second can be affected by a coverage setting at
/// all, and a page carrying only the second could not show that the first is untouched.
fn both_placements(device: &mut Device, program: FunctionId) -> Scene {
    let side = 48.0_f32;
    let square = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(side, side),
        )))
        .expect("upload");
    let triangle = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(side, 0.0)),
            Segment::LineTo(Point::new(0.0, side)),
            Segment::Close,
        ])
        .expect("upload");
    let mut builder = SceneBuilder::new();
    function_fill(&mut builder, square, program, (8.0, 8.0), side);
    function_fill(&mut builder, triangle, program, (8.0, 68.0), side);
    builder.finish()
}

/// **The setting does not reach this paint**, so the two frames are the same bytes.
///
/// Not "within a bound": *equal*. The module comment says where that comes from — the two
/// sites that ask `take_gpu_lane` are both in the solid arm, and a rare paint's coverage
/// tile is `coverage_tile`'s under either setting. `Counters::tiles` is asserted beside the
/// pixels because it is the exact-arithmetic half of the same statement: one tile for the
/// triangle, none for the rectangle, on both devices.
///
/// If this ever fails, the fix is **not** to loosen it to ADR 0016's bound. It means a rare
/// paint has learned the device lane, and then this file's subject changes: the two lanes
/// draw different geometry, and what would need writing is `coverage_lanes.rs`'s
/// comparison for this paint rather than an equality with a wider tolerance.
#[test]
fn the_function_lane_draws_the_same_bytes_under_either_coverage_setting() {
    let frame_of = |coverage: Coverage| {
        let (mut device, adapter) = device(coverage);
        device.wait_until_warm();
        let program = device
            .upload_function(&POSITION_IS_COLOUR)
            .expect("`x y 0.5` is admitted");
        let scene = both_placements(&mut device, program);
        let frame = device
            .render(
                &scene,
                &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
                Target::Readback,
            )
            .expect("the frame is drawn");
        let tiles = frame.counters().tiles;
        (adapter, tiles, frame.into_raster().unwrap().into_pixels())
    };
    let (adapter, cpu_tiles, cpu) = frame_of(Coverage::Cpu);
    let (_, gpu_tiles, gpu) = frame_of(Coverage::Gpu);

    assert_eq!(
        cpu_tiles, 1,
        "{adapter}: the triangle is one coverage tile and the rectangle is none"
    );
    assert_eq!(
        gpu_tiles, cpu_tiles,
        "{adapter}: a function fill's coverage is the CPU rasteriser's under either \
         setting, so the sheet holds the same tiles"
    );
    assert_eq!(
        gpu, cpu,
        "{adapter}: `take_gpu_lane` is asked only in the solid arm, so the setting cannot \
         change a single byte of a page of function paint"
    );

    // And the page is the page, not two blank frames agreeing: the rectangle's own
    // position colours it (§10.7.4), which is what says the fixture drew.
    let inside = pixel(&gpu, 20, 20);
    let want = ((20.5_f32 - 8.0) / 48.0 * 255.0).round() as i32;
    assert!(
        (i32::from(inside[0]) - want).abs() <= 1,
        "{adapter}: the rect-hinted placement carries the function's value at the pixel's \
         centre; expected red {want} ± 1, got {inside:?}"
    );
    assert_eq!(
        inside[3], 255,
        "{adapter}: and it is opaque inside its domain"
    );
}

/// **One sheet, two producers, and a function quad reads its own tile.**
///
/// Under [`Coverage::Gpu`] a solid mark the atlas will not hold has its coverage *drawn* by
/// the device into the frame's scratch sheet, while a function fill's is rasterised on the
/// processor and packed onto the same sheet. `coverage_lanes.rs` holds that case for two
/// solid marks; this is the same seam with the shader that computes its texel as
/// `coverage.xy + p − dest.xy` on one side of it, which is the arithmetic that breaks if a
/// tile is not where the packer said.
///
/// The two marks do not overlap, so each half of the page names one of them — and the same
/// page is drawn under **both** settings, because that is the only thing that proves the
/// blob's coverage really was drawn by the device: a tile count cannot say, since both
/// lanes pack exactly one tile per mark. The blob's antialiased edge **must** move between
/// the settings (ADR 0016's sample grid against the CPU lane's exact area) and the function
/// triangle's pixels **must not**.
#[test]
fn a_function_tile_and_a_device_drawn_tile_share_one_sheet() {
    let (adapter, tiles, gpu) = sheet_frame(Coverage::Gpu);
    let (_, cpu_tiles, cpu) = sheet_frame(Coverage::Cpu);

    assert_eq!(
        tiles, 2,
        "{adapter}: one tile for the blob and one for the function triangle, on one sheet"
    );
    assert_eq!(cpu_tiles, tiles, "{adapter}: and the same two under Cpu");

    // The device really drew the blob's coverage: its edge answers the sample grid rather
    // than the exact area, so the two settings disagree there. Without this the test would
    // be comparing the CPU lane with itself.
    let blob_edge_moved = BLOB_BOX
        .1
        .clone()
        .flat_map(|y| BLOB_BOX.0.clone().map(move |x| (x, y)))
        .any(|(x, y)| pixel(&gpu, x, y) != pixel(&cpu, x, y));
    assert!(
        blob_edge_moved,
        "{adapter}: the solid mark must actually take the device lane under Coverage::Gpu, \
         or the sheet has one producer and this fixture holds nothing"
    );

    // And the function fill's own region is untouched by the setting, byte for byte —
    // including its antialiased hypotenuse, which is where a tile read at the wrong origin
    // would show first.
    for y in FUNCTION_BOX.1.clone() {
        for x in FUNCTION_BOX.0.clone() {
            assert_eq!(
                pixel(&gpu, x, y),
                pixel(&cpu, x, y),
                "{adapter}: ({x}, {y}) is the function fill's, whose coverage is the \
                 processor's under either setting"
            );
        }
    }
    assert_each_mark_read_its_own_tile(&gpu, &adapter);
}

/// The device-drawn tile and the processor-rasterised one, each read where only it can
/// have painted — and one probe outside both, because a tile written at the wrong origin
/// leaves its coverage somewhere.
fn assert_each_mark_read_its_own_tile(gpu: &[u8], adapter: &str) {
    assert_eq!(
        pixel(gpu, 110, 58),
        [0, 0, 0, 255],
        "{adapter}: the middle of the blob"
    );
    // The function's own value at the pixel's centre, which no other mark on this page
    // could have produced.
    let want = |device_coord: u32, origin: f32| {
        ((device_coord as f32 + 0.5 - origin) / 40.0 * 255.0).round() as i32
    };
    for (x, y) in [(12_u32, 134_u32), (20, 142)] {
        let got = pixel(gpu, x, y);
        assert!(
            (i32::from(got[0]) - want(x, 8.0)).abs() <= 1
                && (i32::from(got[1]) - want(y, 130.0)).abs() <= 1,
            "{adapter}: at ({x}, {y}) the function quad must read its own tile; expected \
             ({}, {}) ± 1, got {got:?}",
            want(x, 8.0),
            want(y, 130.0)
        );
        assert_eq!(got[3], 255, "{adapter}: and it is opaque there");
    }
    assert_eq!(
        pixel(gpu, 180, 180),
        [0, 0, 0, 0],
        "{adapter}: neither tile reaches the far corner"
    );
}

/// The device pixels the solid blob occupies, and the ones the function triangle does.
/// Disjoint by construction, which is what lets each half of the page name one lane.
const BLOB_BOX: (std::ops::Range<u32>, std::ops::Range<u32>) = (60..160, 8..108);
/// See [`BLOB_BOX`].
const FUNCTION_BOX: (std::ops::Range<u32>, std::ops::Range<u32>) = (8..48, 130..170);

/// The two-mark page: a solid curve the atlas will not hold, whose coverage the device
/// draws under [`Coverage::Gpu`], and a function-painted triangle whose coverage the
/// processor rasterises under either setting. They land on one sheet and do not overlap.
fn sheet_page(device: &mut Device, program: FunctionId) -> Scene {
    // Far larger than TINY_ATLAS admits, placed once — so `take_gpu_lane`'s cache condition
    // and its triangle floor both say the device should draw its coverage.
    let side = 100.0_f32;
    let blob = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.0, side * 0.5)),
            Segment::CubicTo {
                c1: Point::new(0.0, 0.0),
                c2: Point::new(side, 0.0),
                to: Point::new(side, side * 0.5),
            },
            Segment::CubicTo {
                c1: Point::new(side, side),
                c2: Point::new(0.0, side),
                to: Point::new(0.0, side * 0.5),
            },
            Segment::Close,
        ])
        .expect("upload");
    let triangle = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(40.0, 0.0)),
            Segment::LineTo(Point::new(0.0, 40.0)),
            Segment::Close,
        ])
        .expect("upload");
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            blob,
            Affine::translate(60.0, 8.0),
            FillRule::NonZero,
            Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0)),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("a valid solid fill");
    function_fill(&mut builder, triangle, program, (8.0, 130.0), 40.0);
    builder.finish()
}

/// [`sheet_page`] drawn on a device set to `coverage`: the adapter's name, the sheet's tile
/// count and the frame's pixels.
fn sheet_frame(coverage: Coverage) -> (String, u32, Vec<u8>) {
    let (mut device, adapter) = device(coverage);
    device.wait_until_warm();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let scene = sheet_page(&mut device, program);
    let frame = device
        .render(
            &scene,
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the frame is drawn");
    let tiles = frame.counters().tiles;
    (adapter, tiles, frame.into_raster().unwrap().into_pixels())
}

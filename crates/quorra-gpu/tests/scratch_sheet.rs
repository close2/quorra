//! The frame's scratch sheet is as wide as it is used, and is charged for what it is.
//!
//! ADR 0021. Coverage tiles are packed onto one R8 sheet whose *packing* width is the
//! device's maximum dimension — narrow it and real pages are refused for capacity
//! (the caller's feedback §3) — but `finish` used to commit a texture of that width.
//! On this machine that is 16 384 texels a row, so a page with one 180-pixel tile
//! allocated and uploaded 2.95 MB to carry 32 KB, and the GPU coverage lane, whose
//! winding texture takes its extent from the same sheet at eight bytes a texel, paid
//! 23.6 MB for it.
//!
//! Two claims, and the second is why this file exists rather than a benchmark:
//!
//! - **Narrowing moves no tile.** Every tile sits left of the widest shelf cursor, so
//!   the region kept is the region written. The pixel tests here render a multi-shelf
//!   scene and compare each fill against the same fill drawn alone.
//! - **The sheet is charged, not only the tiles on it.** Shelf packing leaves gaps and
//!   the gaps are allocated too; before this, the largest scene-derived allocation a
//!   page of path work made was the one number nobody counted, which is the reverse of
//!   what principle 3 asks.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Coverage, Device, Options, RenderError, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, OutlineId, Paint, Point, Scene, SceneBuilder,
    Segment,
};

const W: u32 = 1191;
const H: u32 = 1684;

fn device(coverage: Coverage, budget: u64) -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        coverage,
        max_frame_bytes: budget,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

/// A closed curve `half` wide and `half` tall about (`cx`, `cy`) — past
/// `MAX_GLYPH_DIM` when `half` is large, so its coverage lands on the sheet rather
/// than in the atlas, which is the only case the sheet's extent is about.
fn blob(device: &mut Device, cx: f32, cy: f32, half_w: f32, half_h: f32) -> OutlineId {
    device
        .upload_outline(&[
            Segment::MoveTo(Point::new(cx - half_w, cy)),
            Segment::CubicTo {
                c1: Point::new(cx - half_w, cy - half_h),
                c2: Point::new(cx + half_w, cy - half_h),
                to: Point::new(cx + half_w, cy),
            },
            Segment::CubicTo {
                c1: Point::new(cx + half_w, cy + half_h),
                c2: Point::new(cx - half_w, cy + half_h),
                to: Point::new(cx - half_w, cy),
            },
            Segment::Close,
        ])
        .unwrap()
}

fn fill(builder: &mut SceneBuilder, outline: OutlineId) {
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Solid(Color::new(0.15, 0.3, 0.65, 1.0)),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
}

fn render(device: &mut Device, scene: &Scene) -> (Vec<u8>, u64) {
    let frame = device
        .render(
            scene,
            &Viewport::full(W, H, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders");
    let uploaded = frame.counters().bytes_uploaded;
    (frame.into_raster().unwrap().into_pixels(), uploaded)
}

/// One 180-pixel tile on a page-sized frame moves its own bytes and not the device's
/// maximum dimension's.
#[test]
fn one_tile_does_not_commit_a_row_of_the_device_maximum() {
    let mut device = device(Coverage::Cpu, quorra_gpu::DEFAULT_MAX_FRAME_BYTES);
    let dimension = u64::from(device.limits().max_target_size);
    let outline = blob(&mut device, 300.0, 300.0, 90.0, 90.0);
    let mut builder = SceneBuilder::new();
    fill(&mut builder, outline);
    let (_, uploaded) = render(&mut device, &builder.finish());

    // The tile is about 180x180 = 32 KB. A sheet committed at the packing width would
    // be `dimension x 180` — 2.95 MB on this machine — so anything near that is the
    // defect returning.
    let committed_row = dimension.saturating_mul(180);
    assert!(
        uploaded < committed_row / 4,
        "a single 180-pixel tile uploaded {uploaded} bytes; a sheet committed at the \
         device's {dimension}-texel packing width would have been about {committed_row}"
    );
}

/// The GPU coverage lane takes its winding texture's extent from the same sheet at
/// eight bytes a texel, so the narrowing is worth an order of magnitude there.
#[test]
fn the_gpu_lanes_winding_texture_shrinks_with_the_sheet() {
    let mut device = device(Coverage::Gpu, quorra_gpu::DEFAULT_MAX_FRAME_BYTES);
    let dimension = u64::from(device.limits().max_target_size);
    let outline = blob(&mut device, 300.0, 300.0, 90.0, 90.0);
    let mut builder = SceneBuilder::new();
    fill(&mut builder, outline);
    let (_, uploaded) = render(&mut device, &builder.finish());

    // `dimension x 180 x 8` for the winding target alone would be 23.6 MB here.
    let committed = dimension.saturating_mul(180).saturating_mul(8);
    assert!(
        uploaded < committed / 8,
        "the GPU lane's frame moved {uploaded} bytes; a sheet at the packing width \
         would have cost about {committed} for the winding texture alone"
    );
}

/// Narrowing restrides the written rows, so every tile must still hold exactly the
/// bytes it held: three fills of different heights land on three shelves, and each one
/// must equal itself drawn alone.
#[test]
fn tiles_keep_their_pixels_across_the_narrowing() {
    let mut device = device(Coverage::Cpu, quorra_gpu::DEFAULT_MAX_FRAME_BYTES);
    let shapes = [
        (250.0_f32, 250.0_f32, 100.0_f32, 30.0_f32),
        (700.0, 500.0, 40.0, 120.0),
        (400.0, 900.0, 90.0, 90.0),
    ];
    let outlines: Vec<OutlineId> = shapes
        .iter()
        .map(|&(cx, cy, w, h)| blob(&mut device, cx, cy, w, h))
        .collect();

    let mut together = SceneBuilder::new();
    for outline in &outlines {
        fill(&mut together, *outline);
    }
    let (all, _) = render(&mut device, &together.finish());

    for (index, outline) in outlines.iter().enumerate() {
        let mut alone = SceneBuilder::new();
        fill(&mut alone, *outline);
        let (one, _) = render(&mut device, &alone.finish());
        // The shapes do not overlap, so every pixel of each one must agree.
        let (cx, cy, half_w, half_h) = shapes[index];
        for dy in -(half_h as i32)..(half_h as i32) {
            let y = (cy as i32 + dy) as u32;
            for dx in -(half_w as i32)..(half_w as i32) {
                let x = (cx as i32 + dx) as u32;
                let at = ((y * W + x) * 4) as usize;
                assert_eq!(
                    all[at..at + 4],
                    one[at..at + 4],
                    "shape {index} at ({x}, {y}) differs between the packed sheet and \
                     the same fill drawn alone"
                );
            }
        }
    }
}

/// The gaps shelf packing leaves are allocated bytes, and the budget now says so: a
/// frame whose tiles are small but whose sheet is not is refused by the sheet.
#[test]
fn the_sheet_is_charged_and_not_only_the_tiles_on_it() {
    // A wide flat tile and a tall narrow one: two shelves, so the sheet is the wide
    // one's width by the sum of the heights — several times the tiles' own area.
    let tiles: u64 = 300 * 24 + 24 * 300;
    let sheet: u64 = 300 * (24 + 300);
    assert!(
        sheet > tiles * 2,
        "the fixture must leave a real gap to charge: {sheet} against {tiles}"
    );

    // A budget between the two: enough for every tile, not enough for the sheet.
    let mut device = device(Coverage::Cpu, tiles * 2);
    let wide = blob(&mut device, 300.0, 300.0, 150.0, 12.0);
    let tall = blob(&mut device, 800.0, 800.0, 12.0, 150.0);
    let mut builder = SceneBuilder::new();
    fill(&mut builder, wide);
    fill(&mut builder, tall);

    match device.render(
        &builder.finish(),
        &Viewport::full(W, H, Affine::IDENTITY),
        Target::Readback,
    ) {
        Err(RenderError::FrameBudgetExceeded { needed, budget }) => {
            assert!(
                needed > budget,
                "the refusal must name the sheet\'s own bytes: {needed} against {budget}"
            );
        }
        other => panic!("expected the sheet\'s gaps to be charged, got {other:?}"),
    }
}

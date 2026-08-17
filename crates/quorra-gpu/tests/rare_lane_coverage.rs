//! Which paints [`Coverage`] reaches, held in the pixels for the shading arm (ADR 0064).
//!
//! # The statement
//!
//! `Options::coverage` chooses who rasterises coverage — the processor's scanline
//! rasteriser (ADR 0008) or the device's winding accumulation (ADR 0016) — **for a solid
//! fill or stroke, and for nothing else.** `Encoder::take_gpu_lane` is consulted in
//! exactly two places, `encode/fill.rs`'s `fill_solid` and `encode/coverage.rs`'s
//! `push_coverage_styled`, and both are the solid arm; a shading, a mesh, an image or a
//! §7.10.5 program reaches the sheet through `encode/rare.rs`'s `push_rare_coverage`,
//! which calls `coverage_tile` directly and never asks.
//!
//! ADR 0064 measured what leaving it that way costs — **0.11 % of the caller's corpus's
//! rasterised coverage at scale 1 and 0.63 % at 4×** — and decided to leave it, so the
//! omission stopped being an oversight nobody had priced and became a decision. A
//! decision has to be held to, which is why this file exists.
//!
//! # Why this file, when `tests/function_coverage.rs` already asserts it
//!
//! That file asserts it for **one** of the four paints the claim names, `Paint::Function`,
//! and through **one** of the two doors, `encode_fill`'s. The public claim on [`Coverage`]
//! is about every non-solid paint, and `encode_stroke`'s non-solid arm is the door the
//! measured population actually arrives through: 209 of the corpus's 559 rare-painted
//! coverage tiles are not under a residue clip, and the pages that carry the largest share
//! of them are pattern-painted *text* and *strokes* (`doc/notes-rare-lane.md` §4). An
//! assertion that covers a quarter of what it claims is the shape `tests/shader_copies.rs`
//! was found in — it named 8 shaders where the tree had 10, compared five, and passed.
//!
//! # Why the equality is an equality and not a bound
//!
//! `tests/coverage_lanes.rs` bounds the two *lanes* at an eighth of a pixel for the sample
//! grid plus a quarter for the processor lane's flattening. That is not the bound here:
//! the setting does not reach this paint at all, so the two frames are the **same bytes**.
//! If this ever fails, the fix is not to loosen it to ADR 0016's bound — it means the rare
//! lane has learned the device path, which is ADR 0064 being reopened, and what would then
//! need writing is `coverage_lanes.rs`'s comparison for these paints.
//!
//! # The control, and why an equality needs one
//!
//! "The two frames are equal" reads the same on a device where nothing takes the device
//! lane at all — a fixture comparing one lane with itself, which is the trap
//! `doc/HANDOVER.md` records from `m45.rs`. So
//! [`the_setting_does_reach_the_same_geometry_when_its_paint_is_solid`] draws the *same two
//! shapes* with a solid paint and asserts the settings **disagree** there. The two tests
//! are one statement: the setting reaches this geometry, and does not reach this paint.

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
    Affine, BlendMode, Color, Compose, FillRule, LineCap, LineJoin, Paint, Point, Scene,
    SceneBuilder, Segment, ShadingKind, Stop, Stroke,
};

/// The target the pages below are drawn into. 320 × 4 = 1 280 bytes a row, a multiple of
/// the 256-byte buffer-copy alignment.
const SIZE: u32 = 320;

/// A 4 KiB atlas is 64 × 64, so neither mark below fits an entry and the cache condition
/// of `take_gpu_lane` cannot decline on their behalf. Without it the solid control would
/// take the processor lane under both settings and would assert nothing.
const TINY_ATLAS: u64 = 4 * 1024;

/// The marks' device geometry, chosen so that `take_gpu_lane`'s cost comparison passes:
/// a tile of `BAR_WIDTH × BAR_HEIGHT` is 18 000 coverage bytes against roughly six
/// flattened points at `3 × WindingVertex::STRIDE` each — 576 bytes of triangles. A short
/// mark would fail that comparison and the control would compare one lane with itself.
const BAR_WIDTH: f32 = 60.0;
/// See [`BAR_WIDTH`].
const BAR_HEIGHT: f32 = 300.0;

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

/// A bar at `left`, described with **five** points so that `axis_aligned_rect` — which
/// accepts exactly one closed subpath of four corners — refuses it and the mark needs a
/// rasterised coverage tile. The same device that `tests/thin_marks.rs` uses to keep a
/// mark out of ADR 0007's analytic lane.
///
/// **Its edges do not lie on pixel boundaries**, and that is load-bearing for the control:
/// where no edge crosses a pixel the two lanes agree *exactly* (ADR 0016), so a bar at
/// integer coordinates draws the same bytes on both and the control below asserts nothing.
/// It was written that way first, and the control is what caught it.
fn bar(left: f32) -> Vec<Segment> {
    let top = 10.3;
    vec![
        Segment::MoveTo(Point::new(left, top)),
        Segment::LineTo(Point::new(left + BAR_WIDTH, top)),
        // Collinear, and the whole of the difference from a rectangle.
        Segment::LineTo(Point::new(left + BAR_WIDTH, top + BAR_HEIGHT * 0.5)),
        Segment::LineTo(Point::new(left + BAR_WIDTH, top + BAR_HEIGHT)),
        Segment::LineTo(Point::new(left, top + BAR_HEIGHT)),
        Segment::Close,
    ]
}

/// The stroked mark: one long segment, stroked wide enough that its expansion is a tile of
/// the same order as the filled bar's.
fn rule(left: f32) -> Vec<Segment> {
    vec![
        Segment::MoveTo(Point::new(left, 10.3)),
        Segment::LineTo(Point::new(left, 10.3 + BAR_HEIGHT)),
    ]
}

fn wide_stroke() -> Stroke {
    Stroke {
        width: BAR_WIDTH,
        cap: LineCap::Butt,
        join: LineJoin::Miter,
        miter_limit: 10.0,
    }
}

/// The two marks' device columns, disjoint so each half of the page names one of them.
const FILL_COLUMNS: std::ops::Range<u32> = 20..80;
/// See [`FILL_COLUMNS`].
const STROKE_COLUMNS: std::ops::Range<u32> = 190..250;

/// The page both tests draw: one non-rect-hinted fill and one stroke, under `paint`.
///
/// Both doors to `push_rare_coverage` are on it — `encode_fill`'s non-rect-hinted arm and
/// `encode_stroke`'s non-solid arm — because the claim under test is about the paint and
/// not about which command carried it.
fn page(device: &mut Device, paint: Paint) -> Scene {
    let filled = device.upload_outline(&bar(20.3)).expect("upload");
    let stroked = device.upload_outline(&rule(220.3)).expect("upload");
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            filled,
            Affine::IDENTITY,
            FillRule::NonZero,
            paint,
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("a valid fill");
    builder
        .stroke(
            stroked,
            Affine::IDENTITY,
            wide_stroke(),
            paint,
            None,
            BlendMode::Normal,
            None,
        )
        .expect("a valid stroke");
    builder.finish()
}

/// A black-to-white axial sweep across the target, so that a mark's colour varies over its
/// own tile: a paint of one flat colour could not tell a tile read at the wrong origin from
/// one read at the right one.
fn axial(device: &mut Device) -> Paint {
    let ramp = device
        .upload_ramp(&[
            Stop {
                offset: 0.0,
                color: Color::new(0.0, 0.0, 0.0, 1.0),
            },
            Stop {
                offset: 1.0,
                color: Color::new(1.0, 1.0, 1.0, 1.0),
            },
        ])
        .expect("a two-stop ramp is admitted");
    Paint::Shading {
        ramp,
        kind: ShadingKind::Axial {
            start: Point::new(0.0, 0.0),
            end: Point::new(SIZE as f32, 0.0),
            extend: (true, true),
        },
        transform: Affine::IDENTITY,
    }
}

/// One frame of `page` under `coverage`: the adapter's name, the sheet's tile count and the
/// pixels.
fn frame_of(coverage: Coverage, solid: bool) -> (String, u32, Vec<u8>) {
    let (mut device, adapter) = device(coverage);
    device.wait_until_warm();
    let paint = if solid {
        Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0))
    } else {
        axial(&mut device)
    };
    let scene = page(&mut device, paint);
    let frame = device
        .render(
            &scene,
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the frame is inside every budget");
    let tiles = frame.counters().tiles;
    (adapter, tiles, frame.into_raster().unwrap().into_pixels())
}

/// How many pixels of the two frames differ inside `columns`.
fn differing(a: &[u8], b: &[u8], columns: std::ops::Range<u32>) -> u32 {
    let mut count = 0;
    for y in 0..SIZE {
        for x in columns.clone() {
            let at = ((y * SIZE + x) * 4) as usize;
            if a[at..at + 4] != b[at..at + 4] {
                count += 1;
            }
        }
    }
    count
}

/// **The setting does not reach a shading paint**, through either door, so the two frames
/// are the same bytes — and the sheet holds the same two tiles.
///
/// `Counters::tiles` is asserted beside the pixels because it is the exact-arithmetic half
/// of the same statement (`doc/HANDOVER.md`: a claim about "how many" is a count and is
/// exact): both lanes seat one tile per mark, so an equal count is not on its own evidence
/// that the same lane ran — which is what the control below is for.
#[test]
fn the_shading_lane_draws_the_same_bytes_under_either_coverage_setting() {
    let (adapter, cpu_tiles, cpu) = frame_of(Coverage::Cpu, false);
    let (_, gpu_tiles, gpu) = frame_of(Coverage::Gpu, false);

    assert_eq!(
        cpu_tiles, 2,
        "{adapter}: the fill is one coverage tile and the stroke's expansion is another"
    );
    assert_eq!(
        gpu_tiles, cpu_tiles,
        "{adapter}: a shaded mark's coverage is the processor's under either setting, so \
         the sheet holds the same tiles"
    );

    let fill_moved = differing(&cpu, &gpu, FILL_COLUMNS);
    let stroke_moved = differing(&cpu, &gpu, STROKE_COLUMNS);
    assert_eq!(
        (fill_moved, stroke_moved),
        (0, 0),
        "{adapter}: `take_gpu_lane` is asked only in the solid arm, so the setting cannot \
         change a byte of a shaded fill ({fill_moved} pixels moved) or a shaded stroke \
         ({stroke_moved} moved). ADR 0064 is the decision this holds; reopening it means \
         rewriting this file's subject, not widening its tolerance"
    );

    // And the page is a page rather than two blank frames agreeing.
    let ink = (0..SIZE)
        .filter(|x| gpu[((SIZE / 2 * SIZE + x) * 4 + 3) as usize] > 0)
        .count();
    assert!(
        ink >= (BAR_WIDTH as usize) * 2,
        "{adapter}: the fixture drew nothing to compare — {ink} inked pixels across the \
         middle row where two marks {BAR_WIDTH} wide are expected"
    );
}

/// **The control**: the same two shapes with a solid paint, where the setting *does* reach
/// them and the frames must differ.
///
/// Without this, the equality above would pass on a device where nothing ever takes the
/// device lane — one lane compared with itself, which is `m45.rs`'s recorded trap. The
/// difference asserted is ADR 0016's: the device lane answers a 4 × 4 ordered sample grid
/// where the processor lane computes the exact area, so the marks' antialiased edges move.
#[test]
fn the_setting_does_reach_the_same_geometry_when_its_paint_is_solid() {
    let (adapter, _, cpu) = frame_of(Coverage::Cpu, true);
    let (_, _, gpu) = frame_of(Coverage::Gpu, true);

    let fill_moved = differing(&cpu, &gpu, FILL_COLUMNS);
    let stroke_moved = differing(&cpu, &gpu, STROKE_COLUMNS);
    assert!(
        fill_moved > 0 && stroke_moved > 0,
        "{adapter}: these shapes must take the device lane under Coverage::Gpu when their \
         paint is solid, or the equality this file asserts for a shading is an equality \
         between one lane and itself. Fill moved {fill_moved} pixels, stroke {stroke_moved}"
    );
}

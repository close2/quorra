//! ADR 0090's per-frame hybrid: under `Coverage::Cpu`, tiles the atlas will not hold
//! flatten on the device — and the reroute is invisible at the pixel.
//!
//! The claim rests on the same identity `tests/compute_lane.rs` holds the whole
//! compute lane to: the device's flattening and deposit produce the CPU scanline's
//! bytes. This suite forces the hybrid on over the CI's software adapter — where the
//! auto rule would keep it off, for time and not for pixels — and compares whole
//! frames byte for byte.

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

use quorra_gpu::{Coverage, Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, Paint, Point, Scene, SceneBuilder, Segment,
};

const SIZE: u32 = 128;

/// An atlas the glyphs fit and the big fill does not: the hybrid's population is
/// what the atlas *refuses*, so the fixture has to make it refuse (the same
/// construction `compute_lane.rs` uses for the same reason).
const SMALL_ATLAS: u64 = 2048;

fn device(assist: bool) -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        coverage: Coverage::Cpu,
        compute_assist: Some(assist),
        atlas_budget: SMALL_ATLAS,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

/// A page with both populations: one small outline placed many times (the atlas's),
/// and large one-off fills (the scratch lane's, which the hybrid reroutes).
fn page(device: &mut Device) -> Scene {
    let glyph = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(6.0, 1.0)),
            Segment::CubicTo {
                c1: Point::new(7.0, 4.0),
                c2: Point::new(4.0, 7.0),
                to: Point::new(1.0, 6.0),
            },
            Segment::Close,
        ])
        .unwrap();
    let big = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(10.0, 70.0)),
            Segment::LineTo(Point::new(110.0, 62.0)),
            Segment::CubicTo {
                c1: Point::new(120.0, 90.0),
                c2: Point::new(60.0, 122.0),
                to: Point::new(14.0, 104.0),
            },
            Segment::Close,
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    for index in 0..24u32 {
        let (col, row) = (index % 8, index / 8);
        builder
            .fill(
                glyph,
                Affine {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    e: 8.0 + 14.0 * col as f32,
                    f: 8.0 + 14.0 * row as f32,
                },
                FillRule::NonZero,
                Paint::Solid(Color::new(0.1, 0.1, 0.4, 1.0)),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
    }
    builder
        .fill(
            big,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Solid(Color::new(0.7, 0.3, 0.1, 0.9)),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    builder.finish()
}

/// The identity itself, whole-frame: the hybrid moves work between lanes and not one
/// byte of the picture.
#[test]
fn the_hybrid_changes_no_pixel() {
    let render = |assist: bool| {
        let mut device = device(assist);
        let scene = page(&mut device);
        device
            .render(
                &scene,
                &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
                Target::Readback,
            )
            .expect("renders")
            .into_raster()
            .unwrap()
            .into_pixels()
    };
    assert_eq!(
        render(false),
        render(true),
        "Cpu↔Compute are held to zero pixels, so the reroute must be too"
    );
}

/// The routing claim behind the time: with the hybrid on, the compute lane's
/// dispatch runs (its named spans appear in the frame's phases) and the repeated
/// glyph still reaches the atlas; with it off, the lane never wakes.
#[test]
fn the_big_fill_takes_the_device_and_the_glyphs_keep_the_atlas() {
    let ran_compute = |assist: bool| {
        let mut device = device(assist);
        let scene = page(&mut device);
        let frame = device
            .render(
                &scene,
                &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
                Target::Readback,
            )
            .expect("renders");
        let compute = frame
            .timings()
            .phases
            .iter()
            .any(|(name, _)| *name == "compute count stall");
        let glyphs = frame.counters().lanes.glyph;
        (compute, glyphs)
    };
    let (compute, glyphs) = ran_compute(true);
    assert!(compute, "the one-off fill flattens on the device");
    assert!(glyphs > 0, "the repeated glyph stays cached: {glyphs}");
    let (compute, _) = ran_compute(false);
    assert!(
        !compute,
        "off is off: the software adapter's scanline keeps the tile"
    );
}

/// ADR 0093's increment: a tile the packer has no room for is declined at admission
/// — before a worker rasterises it — and the hybrid sends it to the device instead
/// of the sheet. The population is distinct one-off glyph-sized tiles overflowing a
/// small atlas: without the reroute they fall through at commit (the overflow
/// counter says so); with it they never enter, and the picture is the compute
/// lane's own — the exact phase, where the fall-through drew the quantised one that
/// bought nothing.
#[test]
#[allow(clippy::too_many_lines)] // one claim, three constructions, read top to bottom
fn a_full_atlas_declines_at_admission_and_the_device_draws_the_tile() {
    let scene_of = |device: &mut Device| {
        let mut builder = SceneBuilder::new();
        // 40 distinct tiny outlines: each is cacheable by size, collectively far
        // past SMALL_ATLAS, and each placed twice so the census would cache them.
        for index in 0..40u32 {
            let wobble = (index % 7) as f32;
            let outline = device
                .upload_outline(&[
                    Segment::MoveTo(Point::new(0.0, 0.0)),
                    Segment::LineTo(Point::new(9.0 + wobble * 0.3, 1.0)),
                    Segment::CubicTo {
                        c1: Point::new(10.0 + wobble * 0.2, 5.0),
                        c2: Point::new(5.0, 10.0 + wobble * 0.1),
                        to: Point::new(1.0, 9.0),
                    },
                    Segment::Close,
                ])
                .unwrap();
            for repeat in 0..2u32 {
                builder
                    .fill(
                        outline,
                        Affine {
                            a: 1.0,
                            b: 0.0,
                            c: 0.0,
                            d: 1.0,
                            e: 4.0 + 12.0 * ((index * 2 + repeat) % 10) as f32,
                            f: 4.0 + 15.0 * ((index * 2 + repeat) / 10) as f32,
                        },
                        FillRule::NonZero,
                        Paint::Solid(Color::new(0.2, 0.2, 0.6, 1.0)),
                        None,
                        BlendMode::Normal,
                        Compose::SrcOver,
                        None,
                    )
                    .unwrap();
            }
        }
        builder.finish()
    };
    // Quantum off, in every device of this test: the atlas otherwise rasterises its
    // resident tiles at quantised phases while the compute lane draws exact ones, and
    // the byte comparison below would be measuring that stated policy rather than the
    // reroute. With exact keying all three constructions share one phase.
    let exact_device = |assist: Option<bool>, coverage: Coverage| {
        Device::headless(&Options {
            adapter: Some("llvmpipe".into()),
            coverage,
            compute_assist: assist,
            atlas_budget: SMALL_ATLAS,
            glyph_quantum: None,
            ..Options::default()
        })
        .expect("llvmpipe is present wherever this suite runs")
    };
    let run = |assist: bool| {
        let mut device = exact_device(Some(assist), Coverage::Cpu);
        let scene = scene_of(&mut device);
        let frame = device
            .render(
                &scene,
                &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
                Target::Readback,
            )
            .expect("renders");
        let overflowed = frame.counters().atlas_overflow_tiles;
        (overflowed, frame.into_raster().unwrap().into_pixels())
    };
    let (overflow_without, _) = run(false);
    assert!(
        overflow_without > 0,
        "the fixture overflows the atlas, or the case under test is absent"
    );
    let (overflow_with, rerouted) = run(true);
    assert_eq!(
        overflow_with, 0,
        "with the room probe, a full atlas declines before a worker rasterises"
    );
    // The rerouted picture is the compute lane's own: the exact phase, which is what
    // an uncached tile deserved all along — the fall-through's quantisation bought
    // an entry nobody kept.
    let mut reference = exact_device(None, Coverage::Compute);
    let scene = scene_of(&mut reference);
    let expected = reference
        .render(
            &scene,
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders")
        .into_raster()
        .unwrap()
        .into_pixels();
    // **What may be asserted here, and the finding that bounds it.** The two lanes
    // are *not* byte-identical on this fixture — the pure Cpu frame differs from the
    // pure Compute frame at ~124 bytes (boundary pixels, worst 255), a pre-existing
    // hole in the zero-pixel claim that `tests/compute_lane.rs`'s fixtures never
    // exposed and that is recorded in ADR 0093 as its own follow-up. So the reroute
    // is held to what it actually promises: every byte it changes moves *toward* the
    // compute reference — rerouting removes divergence and can never invent it.
    let (_, kept) = run(false);
    for (index, (with, without)) in rerouted.iter().zip(&kept).enumerate() {
        let reference = expected[index];
        if with != without {
            assert_eq!(
                *with, reference,
                "byte {index}: a byte the reroute changed is the compute lane's own"
            );
        }
    }
    let toward = |frame: &[u8]| frame.iter().zip(&expected).filter(|(a, b)| a != b).count();
    assert!(
        toward(&rerouted) <= toward(&kept),
        "the reroute may only remove divergence from the compute reference"
    );
}

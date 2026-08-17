//! One page at 1×, 2× and 4×, and the property that must hold at each.
//!
//! `doc/notes-ceilings-audit.md` §5 is the round this file witnesses, and the reason it
//! exists is the caller's, from `pdf-viewer/doc/HAYRO_ISSUES_FOR_QUORRA.md` §1 on hayro's
//! `#40`/`#8`/`#63` — three crashes that appear at scale 2 and not at scale 1:
//!
//! > the zoom lane is exactly where this tree exercises quorra hardest […] and a defect
//! > that only appears above 1× is one that a test suite rendering everything at 1×
//! > cannot see.
//!
//! **And this suite did render almost everything at 1×.** Counted on 2026-08-17: of 198
//! test functions that hand a `Viewport` to a render call, 187 use a viewport whose scale
//! is exactly 1 — the two shared helpers (`tests/common/headless.rs`'s `render` and
//! `tests/common/retained.rs`'s `viewport`) are both `Affine::IDENTITY`, and the golden
//! comparison against the independent CPU reference (`m1.rs`) is a y-flip whose diagonal
//! is ±1. Of the eleven that are not, seven are `coverage_lanes.rs` at a single factor of
//! 16 comparing the two lanes *with each other*, two are `perf_gate.rs` at 20× asserting
//! only a cull count, one is `retained_invalidation.rs` at 1.5× asserting only a cache
//! decision, and one is the fuzzer. No test anywhere walked a *range* of scales, and
//! `examples/zoom.rs`, which does, asserts nothing and is never run by CI.
//!
//! # Why a property and not bytes
//!
//! The same page at two magnifications is two different pictures: coverage is an area, so
//! every antialiased edge differs. What does not differ is what the area *is*. Two
//! statements follow from the module definition in `raster.rs` — coverage is the fraction
//! of the pixel the shape covers — and both are checkable at any scale:
//!
//! 1. **Ink is area.** A mark of scene area `A` drawn at magnification `s` covers `A·s²`
//!    device pixels, so the sum of the page's alpha, in units of full coverage, is `A·s²`.
//!    A quantity computed in device space and assumed to be in a range only scale 1
//!    guarantees — which is precisely what `#40` and `#63` were — breaks this.
//! 2. **The mark is where the transform puts it.** The bounding box of the inked pixels is
//!    the scene box times `s`, to within the pixel the edge is rounded out to.
//!
//! Three marks, chosen so that the three paths a zoom stresses are each walked at each
//! scale: a fill (the coverage lane, whose tile is computed in device space and whose
//! flattening tolerance is stated in device pixels), a stroke (whose expansion is the
//! arithmetic `raster::direction` does, and whose device delta grows with `s`), and a fill
//! under a non-rectangular clip (the residue path, which is what the corpus's two
//! refusals at 2× are about — `doc/notes-tiling-ceiling.md`).

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
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, LineCap, LineJoin, OutlineId, Paint, Point, Scene,
    SceneBuilder, Segment, Stroke,
};

/// The page in scene units. Every probe below is stated in these and multiplied by the
/// magnification, so one number describes the fixture at all three scales.
const UNITS: u32 = 64;

/// The magnifications this file walks. 1 is the baseline the rest of the suite already
/// covers; 2 is where the caller's corpus starts refusing; 4 is their separate gate.
const SCALES: [u32; 3] = [1, 2, 4];

fn device() -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

fn black() -> Paint {
    Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0))
}

/// What one page of alpha adds up to, in units of one fully covered pixel, and the
/// bounding box of the pixels that carry any of it.
struct Ink {
    covered: f64,
    box_of_ink: Option<[u32; 4]>,
}

fn measure(pixels: &[u8], side: u32) -> Ink {
    let mut covered = 0.0_f64;
    let mut box_of_ink: Option<[u32; 4]> = None;
    for y in 0..side {
        for x in 0..side {
            let a = pixels[((y * side + x) * 4 + 3) as usize];
            if a == 0 {
                continue;
            }
            covered += f64::from(a) / 255.0;
            box_of_ink = Some(match box_of_ink {
                None => [x, y, x, y],
                Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
            });
        }
    }
    Ink {
        covered,
        box_of_ink,
    }
}

/// Draw `scene` at magnification `scale` into a target `scale` times the page, and
/// measure what it inked.
fn ink_at(device: &mut Device, scene: &Scene, scale: u32) -> Ink {
    let side = UNITS * scale;
    let pixels = device
        .render(
            scene,
            &Viewport::full(side, side, Affine::scale(scale as f32, scale as f32)),
            Target::Readback,
        )
        .expect("the frame draws at every magnification")
        .into_raster()
        .unwrap()
        .into_pixels();
    measure(&pixels, side)
}

/// The shared assertion: ink grows as the square of the magnification, and the mark stays
/// where the transform puts it.
///
/// `area` is the mark's area in scene units, derived by hand at each call site. The
/// tolerance is 1 % of it: an edge pixel's coverage rounds to a byte, so a mark with `p`
/// boundary pixels carries at most `p/510` of error, and `p` grows only as `s` while the
/// area grows as `s²` — so the fit gets *tighter* with scale, and a tolerance that holds
/// at 1× holds at 4×.
fn assert_scales_as_area(device: &mut Device, scene: &Scene, area: f64, expected_box: [f32; 4]) {
    for scale in SCALES {
        let ink = ink_at(device, scene, scale);
        let expected = area * f64::from(scale) * f64::from(scale);
        assert!(
            (ink.covered - expected).abs() <= expected * 0.01,
            "at {scale}× the page carries {:.2} pixels of ink where its area is {expected:.2}",
            ink.covered
        );
        let [x0, y0, x1, y1] = ink.box_of_ink.expect("the mark inked something");
        let s = scale as f32;
        // Rounded out to whole pixels at each end, so one pixel of slack on each side.
        for (got, want, name) in [
            (x0 as f32, expected_box[0] * s, "left"),
            (y0 as f32, expected_box[1] * s, "top"),
            (x1 as f32, expected_box[2] * s - 1.0, "right"),
            (y1 as f32, expected_box[3] * s - 1.0, "bottom"),
        ] {
            assert!(
                (got - want).abs() <= 1.0,
                "at {scale}× the ink's {name} edge is {got} where the transform puts it at {want}"
            );
        }
    }
}

/// A right triangle with legs of 32 scene units on the axes, so its area is 512 by
/// inspection and its bounding box is `(8, 8)`–`(40, 40)`.
///
/// A triangle rather than a rectangle on purpose: `quorra_scene::axis_aligned_rect`
/// recognises a rectangle's four edges and a solid fill of one takes the analytic lane,
/// which rasterises no coverage at all. This file is about the coverage lane's
/// device-space arithmetic, so its shape has to be one the recogniser refuses.
fn triangle(device: &mut Device) -> OutlineId {
    device
        .upload_outline(&[
            Segment::MoveTo(Point::new(8.0, 8.0)),
            Segment::LineTo(Point::new(40.0, 8.0)),
            Segment::LineTo(Point::new(8.0, 40.0)),
            Segment::Close,
        ])
        .unwrap()
}

/// **A fill covers its own area at every magnification.**
///
/// The triangle spans 32 × 32 scene units, so its area is 512 and its device area is
/// `512·s²`. The lane is the coverage lane rather than the analytic rectangle one, which
/// is what makes this a statement about `fill_mask`'s device-space arithmetic.
#[test]
fn a_fill_covers_its_own_area_at_every_magnification() {
    let mut device = device();
    let outline = triangle(&mut device);
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
    assert_scales_as_area(
        &mut device,
        &builder.finish(),
        512.0,
        [8.0, 8.0, 40.0, 40.0],
    );
}

/// **A stroke deposits its own band at every magnification.**
///
/// The stroke's width is stated in device pixels — §4.5's decision, settled upstream — so
/// a caller zooming to `s` passes `s` times the width, and this fixture does the same by
/// building one scene per scale. The band is 32 long and 4 wide with butt caps, so its
/// area is 128 scene units; §8.4.3.3's butt cap adds nothing beyond the endpoints, which
/// is what makes the number exact rather than approximate.
#[test]
fn a_stroke_deposits_its_own_band_at_every_magnification() {
    let mut device = device();
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(16.0, 32.0)),
            Segment::LineTo(Point::new(48.0, 32.0)),
        ])
        .unwrap();
    for scale in SCALES {
        let mut builder = SceneBuilder::new();
        builder
            .stroke(
                outline,
                Affine::IDENTITY,
                Stroke {
                    width: 4.0 * scale as f32,
                    cap: LineCap::Butt,
                    join: LineJoin::Miter,
                    miter_limit: 10.0,
                },
                black(),
                None,
                BlendMode::Normal,
                None,
            )
            .unwrap();
        let ink = ink_at(&mut device, &builder.finish(), scale);
        let expected = 128.0 * f64::from(scale) * f64::from(scale);
        assert!(
            (ink.covered - expected).abs() <= expected * 0.01,
            "at {scale}× the band carries {:.2} pixels of ink where it is {expected:.2}",
            ink.covered
        );
        let [x0, y0, x1, y1] = ink.box_of_ink.expect("the band inked something");
        assert_eq!((x0, x1), (16 * scale, 48 * scale - 1), "at {scale}×");
        assert_eq!((y0, y1), (30 * scale, 34 * scale - 1), "at {scale}×");
    }
}

/// **A fill under a non-rectangular clip covers the intersection at every magnification.**
///
/// The residue path is where the caller's corpus starts refusing at 2× and not at 1×
/// (`doc/notes-tiling-ceiling.md`), and it is the one lane whose *tile* is a chain's
/// device region rather than the mark's own box — so it is the lane where a device-space
/// quantity is most likely to be assumed into a range only scale 1 guarantees.
///
/// The clip is a triangle of area 512 and the fill is a rectangle that contains it, so the
/// intersection is the clip: `512·s²`. The clip's outline is not an axis-aligned
/// rectangle, so it stays a residue rather than collapsing into the resolved clip
/// rectangle.
#[test]
fn a_clipped_fill_covers_the_intersection_at_every_magnification() {
    let mut device = device();
    let clip_outline = triangle(&mut device);
    let covering = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(4.0, 4.0)),
            Segment::LineTo(Point::new(60.0, 6.0)),
            Segment::LineTo(Point::new(60.0, 60.0)),
            Segment::LineTo(Point::new(4.0, 58.0)),
            Segment::Close,
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    let clip = builder
        .clip(clip_outline, Affine::IDENTITY, FillRule::NonZero, None)
        .unwrap();
    builder
        .fill(
            covering,
            Affine::IDENTITY,
            FillRule::NonZero,
            black(),
            Some(clip),
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    assert_scales_as_area(
        &mut device,
        &builder.finish(),
        512.0,
        [8.0, 8.0, 40.0, 40.0],
    );
}

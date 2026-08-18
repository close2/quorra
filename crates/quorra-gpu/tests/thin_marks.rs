//! Marks thinner than a device pixel, held against ISO 32000-2 §10.7.4.
//!
//! The caller's hayro reading list opens its §2 with hayro's #104 closed as "I'm sure
//! those are just conflation artifacts, so not a lot we can do here", and does not accept
//! that; `pdf-viewer/doc/QUORRA_HAIRLINE_MARKS.md` is their standing ask on the subject.
//! This file is our half of it: what a mark thinner than a device pixel does *here*.
//!
//! # What §10.7.4 says, and what it does not
//!
//! The rule, verbatim:
//!
//! > A shape shall be scan-converted by painting any pixel whose half-open square region
//! > intersects the shape, no matter how small the intersection is. This ensures that no
//! > shape ever disappears as a result of unfavourable placement relative to the device
//! > pixel grid, as might happen with other possible scan conversion rules. The area
//! > covered by painted pixels shall always be at least as large as the area of the
//! > original shape. This rule applies both to fill operations and to strokes with
//! > non-zero width. Zero-width strokes may be done in an implementation-defined manner
//! > that may include fewer pixels than the rule implies.
//!
//! with NOTE 1 and the EXAMPLE that follow it:
//!
//! > Normally, the intersection of two regions is defined as the intersection of their
//! > interiors. However, for purposes of scan conversion, a filling region is considered
//! > to intersect every pixel through which its boundary passes, even if the interior of
//! > the filling region is empty.
//!
//! > EXAMPLE A zero-width or zero-height rectangle paints a line 1 pixel wide.
//!
//! **That is not a rule about proportional coverage, and it is worth being exact about
//! it.** Read literally the clause is binary — the pixel is *painted*, at the current
//! colour — and it is stated for a device deciding painted-or-not. Two other clauses say
//! that antialiasing is nevertheless allowed and what it means. §10.7.1's NOTE:
//!
//! > The specifics of the scan conversion algorithm are not defined as part of PDF.
//! > Different implementations can perform scan conversion in different ways; techniques
//! > that are appropriate for one device could be inappropriate for another.
//!
//! and §11.3.7.2's NOTE 1, which is where fractional coverage is given a meaning in the
//! transparency model:
//!
//! > Mathematically, elementary objects have "hard" edges, with a shape value of either
//! > 0.0 or 1.0 at every point. However, when such objects are rasterized to device
//! > pixels, the shape values along the boundaries can be anti-aliased, taking on
//! > fractional values representing fractional coverage of those pixels. When such
//! > anti-aliasing is performed, it is important to treat the fractional coverage as shape
//! > rather than opacity.
//!
//! So the assertions below are the two things §10.7.4 *does* decide, and not the third the
//! question invites:
//!
//! 1. **a mark thinner than a device pixel does not disappear** — the clause's own reason
//!    for the rule, stated in the sentence that carries it;
//! 2. **the ink is at least the shape's own area** — "[t]he area covered by painted pixels
//!    shall always be at least as large as the area of the original shape", read in the
//!    units an antialiased device has;
//! 3. proportionality is **ours** (ADR 0005's exact-area coverage), not the clause's, and
//!    it is asserted as our choice rather than as a normative requirement.
//!
//! # The two lanes do not agree, and the difference is recorded here
//!
//! `Coverage::Cpu` computes the exact area a shape covers of each pixel; `Coverage::Gpu`
//! counts samples on an ordered grid (`Options::coverage_samples`, sixteen on a 4 × 4 grid
//! by default). A mark narrower than the grid's spacing falls **between** the sample
//! columns, and on the device lane it then reads zero — which is the disappearance
//! §10.7.4's rule exists to forbid. The numbers are in
//! [`the_device_lane_lets_a_mark_between_two_sample_columns_vanish`], and
//! `doc/notes-hayro-paints.md` §2 carries them to the owner. No sample count removes the
//! effect; only an area rule does.
//!
//! # What is not here
//!
//! The **abutting**-marks half of conflation — two marks meeting at a shared edge and
//! compositing to less than full coverage — is not tested here and is not solved here. The
//! caller says plainly that their tree has not solved it either.

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

use quorra_gpu::{Coverage, Device, Options, RenderError, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, LineCap, LineJoin, Paint, Point, Rect, Scene,
    SceneBuilder, Segment, Stroke,
};

mod common;

use common::probe::alpha;

/// The target the analytic-lane and stroke fixtures draw into. 64 × 4 = 256 bytes a row,
/// exactly the buffer-copy alignment.
const SIZE: u32 = 64;

/// The target the coverage-lane fixtures draw into.
///
/// It is this large for one reason: `Encoder::take_gpu_lane` takes the device lane only
/// when the tile's area in coverage bytes is at least what the shape's triangles cost
/// (three vertices of `WindingVertex::STRIDE` each), so a mark has to be *long* before the
/// device lane will draw it at all. A short bar would be the processor lane under both
/// settings, and the comparison below would be one lane against itself — the trap
/// `doc/HANDOVER.md` records from `m45.rs`.
/// [`the_device_lane_really_is_what_draws_the_tall_bar`] is the control that says it is
/// not.
const TALL: u32 = 768;

/// The device-space left edge every thin mark in this file starts at.
///
/// Deliberately not on a pixel boundary and not on a quarter either: 0.2 into pixel 20 is
/// between the first and second columns of a 4 × 4 sample grid, which is the "unfavourable
/// placement relative to the device pixel grid" §10.7.4 names.
const LEFT: f32 = 20.2;

/// The sub-pixel widths every fixture sweeps, widest first so a failure reads as "it
/// stopped working below here".
const SUB_PIXEL_WIDTHS: [f32; 6] = [0.75, 0.5, 0.25, 0.125, 0.1, 0.05];

fn device_with(coverage: Coverage) -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        coverage,
        // A small atlas, as in `tests/coverage_lanes.rs`: the marks below must reach the
        // sheet rather than be cached in front of it, since this file is about what the
        // two coverage producers put there.
        atlas_budget: 4 * 1024,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

fn render(device: &mut Device, scene: &Scene, side: u32) -> Vec<u8> {
    device
        .render(
            scene,
            &Viewport::full(side, side, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the frame is inside every budget")
        .into_raster()
        .unwrap()
        .into_pixels()
}

/// Total ink across one row: the sum of the row's alphas, in units of a whole pixel.
///
/// This is the quantity §10.7.4's "area covered" is measured in for a mark that crosses
/// the row once — a vertical bar of width `w` covers `w` of a row's area whatever pixels
/// it lands on, so a single number compares a mark against the clause without depending on
/// where the mark sits.
fn row_ink(pixels: &[u8], side: u32, y: u32) -> f32 {
    (0..side)
        .map(|x| {
            let at = ((y * side + x) * 4) as usize;
            f32::from(pixels[at + 3]) / 255.0
        })
        .sum()
}

/// How many pixels of one row carry any ink at all.
///
/// A count rather than a sum wherever the question is §10.7.4's "no shape ever disappears",
/// which is about presence and not about quantity — and a count is exact where a float sum
/// invites a comparison against zero.
fn inked_pixels(pixels: &[u8], side: u32, y: u32) -> u32 {
    (0..side).filter(|x| alpha(pixels, side, *x, y) > 0).count() as u32
}

fn black() -> Paint {
    Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0))
}

fn solid_fill(device: &mut Device, segments: &[Segment]) -> Scene {
    let outline = device.upload_outline(segments).expect("upload");
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
        .expect("a valid fill");
    builder.finish()
}

/// A thin vertical bar as four corners, which `axis_aligned_rect` recognises — so a solid
/// fill of it takes ADR 0007's analytic rectangle lane and never rasterises coverage.
fn analytic_bar(device: &mut Device, width: f32) -> Scene {
    let rect = Rect::new(
        Point::new(LEFT, 4.0),
        Point::new(LEFT + width, SIZE as f32 - 4.0),
    );
    solid_fill(
        device,
        &[
            Segment::MoveTo(rect.min),
            Segment::LineTo(Point::new(rect.max.x, rect.min.y)),
            Segment::LineTo(rect.max),
            Segment::LineTo(Point::new(rect.min.x, rect.max.y)),
            Segment::Close,
        ],
    )
}

/// A tall thin bar at `left`, described with **five** points so that `axis_aligned_rect` —
/// which accepts exactly one closed subpath of four corners — refuses it. The geometry is
/// identical to a rectangle's; only the lane changes, which is what makes this the fixture
/// for the two coverage producers.
fn rasterised_bar_at(device: &mut Device, left: f32, width: f32) -> Scene {
    let bottom = TALL as f32;
    solid_fill(
        device,
        &[
            Segment::MoveTo(Point::new(left, 0.0)),
            Segment::LineTo(Point::new(left + width, 0.0)),
            // Collinear, and the whole of the difference from `analytic_bar`.
            Segment::LineTo(Point::new(left + width, bottom / 2.0)),
            Segment::LineTo(Point::new(left + width, bottom)),
            Segment::LineTo(Point::new(left, bottom)),
            Segment::Close,
        ],
    )
}

fn rasterised_bar(device: &mut Device, width: f32) -> Scene {
    rasterised_bar_at(device, LEFT, width)
}

/// §10.7.4's two decidable requirements, asserted over one row of a mark of width `w`.
fn assert_the_clause_holds(ink: f32, w: f32, lane: &str) {
    assert!(
        ink > 0.0,
        "{lane}, width {w}: the mark disappeared. §10.7.4: painting any pixel the shape \
         intersects \"ensures that no shape ever disappears as a result of unfavourable \
         placement relative to the device pixel grid\""
    );
    // "The area covered by painted pixels shall always be at least as large as the area of
    // the original shape" — in the units an antialiased device has, with one 8-bit
    // rounding allowed for by ADR 0006's store.
    assert!(
        ink >= w - 1.0 / 255.0,
        "{lane}, width {w}: {ink} of ink where §10.7.4 requires the covered area to be at \
         least the shape's own"
    );
}

/// The analytic rectangle lane: a thin rule is the shape a page of ruled lines is made of,
/// and this is the lane a document's rectangles actually take (ADR 0047).
#[test]
fn an_analytic_mark_thinner_than_a_pixel_keeps_ink_proportional_to_its_width() {
    let mut device = device_with(Coverage::Cpu);
    for w in SUB_PIXEL_WIDTHS {
        let scene = analytic_bar(&mut device, w);
        let pixels = render(&mut device, &scene, SIZE);
        let ink = row_ink(&pixels, SIZE, SIZE / 2);
        assert_the_clause_holds(ink, w, "the analytic lane");
        // And ours on top of the clause's: ADR 0005 chose exact-area coverage, so the ink
        // is the shape's area to one 8-bit rounding. This is a choice, not §10.7.4.
        assert!(
            (ink - w).abs() <= 1.0 / 255.0,
            "the analytic lane, width {w}: {ink} of ink where ADR 0005's exact-area \
             coverage gives {w}"
        );
        // The whole mark is inside one pixel column at these widths, so its neighbours are
        // the absence half of the claim — and the assertion above is the control that
        // something was drawn at all.
        assert_eq!(
            alpha(&pixels, SIZE, 19, SIZE / 2),
            0,
            "width {w}: ink spilled left of the mark's own column"
        );
        assert_eq!(
            alpha(&pixels, SIZE, 21, SIZE / 2),
            0,
            "width {w}: ink spilled right of the mark's own column"
        );
    }
}

/// The processor's scanline rasteriser, on the same widths: the lane every clipped mark and
/// every mark under the default setting takes.
#[test]
fn a_rasterised_mark_thinner_than_a_pixel_keeps_ink_proportional_to_its_width() {
    let mut device = device_with(Coverage::Cpu);
    for w in SUB_PIXEL_WIDTHS {
        let scene = rasterised_bar(&mut device, w);
        let pixels = render(&mut device, &scene, TALL);
        let ink = row_ink(&pixels, TALL, TALL / 2);
        assert_the_clause_holds(ink, w, "the processor lane");
        assert!(
            (ink - w).abs() <= 1.0 / 255.0,
            "the processor lane, width {w}: {ink} of ink where ADR 0005's exact-area \
             coverage gives {w}"
        );
    }
}

/// The control for the two tests below, and the reason [`TALL`] is as large as it is: under
/// `Coverage::Gpu` this fixture really is drawn by the device lane.
///
/// The proof is the one `tests/coverage_lanes.rs` uses — only the device lane allocates the
/// `rgba16float` winding texture, so only the device lane is charged for it. A budget the
/// processor lane fits and the device lane does not separates them by construction, where
/// comparing pixels would be comparing the thing under test.
#[test]
fn the_device_lane_really_is_what_draws_the_tall_bar() {
    // Between the sheet the processor lane allocates for this bar and the bytes the device
    // lane needs on top of it; the refusal below prints the second figure.
    let budget = 4 * 1024;
    let mut device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        atlas_budget: 4 * 1024,
        max_frame_bytes: budget,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs");
    let scene = rasterised_bar(&mut device, 0.5);
    let viewport = Viewport::full(TALL, TALL, Affine::IDENTITY);
    assert!(
        device.render(&scene, &viewport, Target::Readback).is_ok(),
        "the processor lane allocates no winding texture, so it fits this budget"
    );

    device.set_coverage(Coverage::Gpu);
    match device.render(&scene, &viewport, Target::Readback) {
        Err(RenderError::FrameBudgetExceeded { needed, budget: b }) => {
            assert_eq!(b, budget);
            assert!(
                needed > budget,
                "the device lane does make the winding texture: {needed} against {budget}"
            );
        }
        other => panic!(
            "this fixture must reach the device lane under Coverage::Gpu, or the tests \
             below compare one lane with itself; got {other:?}"
        ),
    }
}

/// The width of one cell of the device lane's sample grid, in device pixels.
///
/// `Options::coverage_samples` is sixteen by default, laid out 4 × 4, so the columns of
/// sample points sit a quarter of a pixel apart. Derived from the constant rather than
/// written as `0.25`, because the constant is what would move.
fn sample_column_spacing() -> f32 {
    1.0 / (quorra_gpu::DEFAULT_COVERAGE_SAMPLES as f32).sqrt()
}

/// The device lane, on the widths its sample grid can represent: a mark at least one column
/// spacing wide catches at least one column of samples wherever it sits, so §10.7.4 holds
/// there.
#[test]
fn the_device_lane_meets_the_clause_down_to_its_sample_spacing() {
    let mut device = device_with(Coverage::Gpu);
    let mut checked = 0_u32;
    for w in SUB_PIXEL_WIDTHS {
        if w < sample_column_spacing() {
            continue;
        }
        checked += 1;
        let scene = rasterised_bar(&mut device, w);
        let pixels = render(&mut device, &scene, TALL);
        let ink = row_ink(&pixels, TALL, TALL / 2);
        assert_the_clause_holds(ink, w, "the device lane");
        // The device lane's answer is `k` of `n` samples rather than an area, so the bound
        // is the grid's own step and not ADR 0005's rounding.
        assert!(
            (ink - w).abs() <= sample_column_spacing(),
            "the device lane, width {w}: {ink} of ink, further from the shape's area than \
             one sample column"
        );
    }
    assert!(
        checked >= 3,
        "the sweep no longer contains widths the sample grid can represent, so this test \
         asserted nothing"
    );
}

/// The **upper edge of the recorded gap below**, which is the width every option in
/// `doc/notes-thin-mark-options.md` is priced against: at exactly one sample-column spacing
/// the device lane holds §10.7.4 at *every* sub-pixel position, where a tenth of a pixel
/// holds it at four of ten.
///
/// The boundary is arithmetic rather than empirical, and stating it is the point of the
/// test: `Options::coverage_samples` lays `n` samples on a `√n × √n` grid whose columns sit
/// at `(k + ½)/√n` of a pixel, so consecutive columns are `1/√n` apart — and so is the wrap
/// across a pixel boundary, because the grid is symmetric about the centre. An interval of
/// length `1/√n` therefore contains a column wherever it lands, and a mark that catches a
/// column has ink.
///
/// [`the_device_lane_meets_the_clause_down_to_its_sample_spacing`] asserts widths at or
/// above the spacing at **one** position; this asserts that the position is not what carries
/// them, which is what any rule keyed on this width would rely on.
#[test]
fn at_the_sample_spacing_the_device_lane_holds_the_clause_at_every_position() {
    let w = sample_column_spacing();
    let mut device = device_with(Coverage::Gpu);
    for step in 0..10 {
        let left = 20.0 + step as f32 / 10.0;
        let scene = rasterised_bar_at(&mut device, left, w);
        let ink = row_ink(&render(&mut device, &scene, TALL), TALL, TALL / 2);
        assert_the_clause_holds(ink, w, &format!("the device lane at {left}"));
        // And no further from the shape's area than the grid's own step, which is the
        // bound the neighbouring test states for this lane.
        assert!(
            (ink - w).abs() <= sample_column_spacing(),
            "the device lane at {left}: {ink} of ink for a mark of {w}, further from the \
             shape's area than one sample column"
        );
    }
}

/// **A recorded gap, not a claim that this is right.** Below one sample-column spacing the
/// device lane's mark can fall entirely between two columns of sample points and read zero
/// — which is the disappearance §10.7.4's rule exists to forbid:
///
/// > This ensures that no shape ever disappears as a result of unfavourable placement
/// > relative to the device pixel grid, as might happen with other possible scan conversion
/// > rules.
///
/// The processor lane, on the same widths and the same positions, reads the shape's exact
/// area. This test exists so the gap is visible and so a fix moves a gate rather than
/// nothing; `doc/notes-hayro-paints.md` §2 carries the numbers, and no sample count removes
/// the effect — halving the spacing halves the width at which it starts.
#[test]
fn the_device_lane_lets_a_mark_between_two_sample_columns_vanish() {
    let thin = 0.1_f32;
    assert!(
        thin < sample_column_spacing(),
        "this test is about a mark narrower than the sample grid's step"
    );

    let mut processor = device_with(Coverage::Cpu);
    let mut sampled = device_with(Coverage::Gpu);
    // Ten sub-pixel positions across one pixel: the "unfavourable placement" is a property
    // of where the mark lands, so a single position would say nothing about how often.
    let mut vanished = 0_u32;
    for step in 0..10 {
        let left = 20.0 + step as f32 / 10.0;
        let scene = rasterised_bar_at(&mut processor, left, thin);
        let cpu_ink = row_ink(&render(&mut processor, &scene, TALL), TALL, TALL / 2);
        assert!(
            (cpu_ink - thin).abs() <= 1.0 / 255.0,
            "the processor lane holds the clause at every position: {cpu_ink} at {left}"
        );

        let scene = rasterised_bar_at(&mut sampled, left, thin);
        if inked_pixels(&render(&mut sampled, &scene, TALL), TALL, TALL / 2) == 0 {
            vanished += 1;
        }
    }
    assert!(
        vanished > 0,
        "the gap this test records has closed on its own — read it, then delete it or \
         turn it into the requirement"
    );
    assert!(
        vanished < 10,
        "the mark vanished at every one of ten positions, which is a different and worse \
         defect than the one recorded here"
    );
}

/// Their #1023, and the property `Stroke::width`'s doc comment states: the width arrives
/// **already resolved into device pixels** (§4.5 of the brief, from ISO 32000-2 §8.4.3.2
/// with §10.7.5), so it is exactly that wide whatever the viewport does to the geometry.
///
/// The outline is divided by the scale so that the same device line is drawn three times;
/// if the width followed the viewport, the ink would be `w × scale` and this would read
/// four times over at scale 4.
#[test]
fn a_stroke_is_exactly_its_stated_device_width_at_every_scale() {
    let mut device = device_with(Coverage::Cpu);
    for scale in [1.0_f32, 2.0, 4.0] {
        for w in [3.0_f32, 1.0, 0.75, 0.5, 0.25, 0.1] {
            let outline = device
                .upload_outline(&[
                    Segment::MoveTo(Point::new(LEFT / scale, 1.0 / scale)),
                    Segment::LineTo(Point::new(LEFT / scale, (SIZE as f32 - 1.0) / scale)),
                ])
                .expect("upload");
            let mut builder = SceneBuilder::new();
            builder
                .stroke(
                    outline,
                    Affine::IDENTITY,
                    Stroke {
                        width: w,
                        cap: LineCap::Butt,
                        join: LineJoin::Miter,
                        miter_limit: 10.0,
                    },
                    black(),
                    None,
                    BlendMode::Normal,
                    None,
                )
                .expect("a valid stroke");
            let pixels = device
                .render(
                    &builder.finish(),
                    &Viewport::full(SIZE, SIZE, Affine::scale(scale, scale)),
                    Target::Readback,
                )
                .expect("the frame is inside every budget")
                .into_raster()
                .unwrap()
                .into_pixels();
            let ink = row_ink(&pixels, SIZE, SIZE / 2);
            assert_the_clause_holds(ink, w, "a stroke");
            assert!(
                (ink - w).abs() <= 2.0 / 255.0,
                "a stroke of stated device width {w} at viewport scale {scale} laid down \
                 {ink} of ink; the width is device-space and must not follow the viewport"
            );
        }
    }
}

/// Where a mark finally does disappear on **both** lanes, and why: ADR 0006 stores a
/// frame's alpha in eight bits, so a mark whose coverage rounds below half a level has no
/// representation at all.
///
/// This is a cost written down rather than a requirement met — §10.7.4 admits no floor —
/// and the number is here so that a change to the store moves a gate. `1/510` of a device
/// pixel is four thousandths of a point at 96 dpi; the caller's `split_collapsed_fill`
/// snaps the marks that reach this region for a *different* reason (a band at a fractional
/// position renders unevenly), so nothing observed on a real page is known to land here.
#[test]
fn the_eight_bit_store_is_the_floor_under_which_a_mark_has_no_ink() {
    let mut device = device_with(Coverage::Cpu);
    // Just above half a level of 255: representable, and the clause holds.
    let above = 1.0 / 500.0;
    let scene = analytic_bar(&mut device, above);
    let ink = row_ink(&render(&mut device, &scene, SIZE), SIZE, SIZE / 2);
    assert!(
        ink > 0.0,
        "a mark of {above} of a pixel still rounds to a level of the store, so it must be \
         drawn: got {ink}"
    );

    // Just below it: no level of an 8-bit store is nearer than zero.
    let below = 1.0 / 520.0;
    let scene = analytic_bar(&mut device, below);
    let inked = inked_pixels(&render(&mut device, &scene, SIZE), SIZE, SIZE / 2);
    assert_eq!(
        inked, 0,
        "the floor has moved: a mark of {below} of a pixel now has ink in {inked} pixels, \
         which means the store changed and this test's constant is stale rather than wrong"
    );
}

//! What a fill deposits where its shape encloses no area: ISO 32000-2 §10.7.4.
//!
//! > A shape shall be scan-converted by painting any pixel whose half-open square region
//! > intersects the shape, no matter how small the intersection is. This ensures that no
//! > shape ever disappears as a result of unfavourable placement relative to the device
//! > pixel grid […] A zero-width or zero-height rectangle paints a line 1 pixel wide.
//! > (ISO 32000-2 §10.7.4)
//!
//! `848 1085 10159 0 re f` is how a real corpus document rules every line of its grid,
//! and an area rule computes that shape's coverage as zero at every pixel. Until this
//! suite's subject the caller split such subpaths out *before* the scene — which worked,
//! and cost them the scene: the mark's pixel row is the viewport's, so a scene holding
//! pre-split marks was true at exactly one placement. The collapse table is resident on
//! the outline now and the encode places the marks per viewport, which is what these
//! tests hold: **the same scene**, rendered under different viewport affines, puts the
//! mark in the row each placement's own arithmetic names.
//!
//! The arithmetic mirrors the caller's `pdf_render::collapsed` statement for statement
//! (the fill encode's doc says how, and ADR 0086 why); their `collapsed.rs` unit tests
//! assert the split geometry, these assert the pixels.

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

use common::headless::{device, pixels};
use quorra_gpu::{Device, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, Paint, Point, Scene, SceneBuilder, Segment,
};

/// 64 pixels wide: 64 × 4 bytes = 256, the buffer-copy row alignment.
const SIZE: u32 = 64;

/// The mark's paint. Opaque, so a painted pixel reads 255 in alpha and an unpainted
/// one 0 — nothing hides in a rounding tolerance.
const INK: Color = Color::new(0.8, 0.1, 0.1, 1.0);

/// `x y w 0 re`: a rectangle of no height along `y`, from x = 10 to x = 54.
fn zero_height_rectangle(y: f32) -> [Segment; 5] {
    [
        Segment::MoveTo(Point::new(10.0, y)),
        Segment::LineTo(Point::new(54.0, y)),
        Segment::LineTo(Point::new(54.0, y)),
        Segment::LineTo(Point::new(10.0, y)),
        Segment::Close,
    ]
}

/// One scene: the outline filled solid under the non-zero rule, nothing else.
fn filled(device: &mut Device, outline: &[Segment]) -> Scene {
    let id = device.upload_outline(outline).unwrap();
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            id,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Solid(INK),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    builder.finish()
}

/// Straight-alpha bytes of `scene` under `viewport`'s affine.
fn rendered(device: &mut Device, scene: &Scene, transform: Affine) -> Vec<u8> {
    pixels(
        device
            .render(
                scene,
                &Viewport::full(SIZE, SIZE, transform),
                Target::Readback,
            )
            .expect("renders"),
    )
}

/// The alpha at (x, y).
fn alpha(bytes: &[u8], x: u32, y: u32) -> u8 {
    bytes[((y * SIZE + x) * 4 + 3) as usize]
}

/// Which single row of the column at `x` carries ink, asserting there is exactly one.
fn inked_row(bytes: &[u8], x: u32) -> u32 {
    let rows: Vec<u32> = (0..SIZE).filter(|&y| alpha(bytes, x, y) > 0).collect();
    assert_eq!(
        rows.len(),
        1,
        "a snapped mark is one whole pixel row, found rows {rows:?}"
    );
    assert_eq!(alpha(bytes, x, rows[0]), 255, "the whole pixel, not a band");
    rows[0]
}

/// The rule itself: a rectangle of no height marks the whole pixel row it lies in,
/// at whatever fraction of a row it happens to sit — §10.7.4 identifies the row by
/// flooring the device coordinate.
#[test]
fn a_rectangle_of_no_height_paints_the_pixel_row_it_lies_in() {
    let mut device = device();
    let scene = filled(&mut device, &zero_height_rectangle(50.3));
    let bytes = rendered(&mut device, &scene, Affine::IDENTITY);
    assert_eq!(inked_row(&bytes, 32), 50);
    assert_eq!(
        alpha(&bytes, 8, 50),
        0,
        "the mark keeps the subpath's extent"
    );
    assert_eq!(alpha(&bytes, 56, 50), 0);
}

/// A line exactly on a pixel boundary belongs to the pixel above it, not to both:
/// §10.7.4's pixel region "includes its lower but not its upper boundaries". A band
/// centred on the line would straddle the boundary and put half its ink in row 49.
#[test]
fn a_line_on_a_pixel_boundary_takes_the_pixel_above_it() {
    let mut device = device();
    let scene = filled(&mut device, &zero_height_rectangle(50.0));
    let bytes = rendered(&mut device, &scene, Affine::IDENTITY);
    assert_eq!(inked_row(&bytes, 32), 50);
}

/// The claim the resident table exists for: **one scene, three viewports, three
/// rows** — each found by flooring under that placement, none by rebuilding the
/// scene. The caller's pre-split marks could never do this, and it is why their
/// worst page rebuilt its scene on every zoom step.
#[test]
fn the_same_scene_marks_the_row_each_viewport_names() {
    let mut device = device();
    let scene = filled(&mut device, &zero_height_rectangle(50.6));
    for (transform, row) in [
        (Affine::IDENTITY, 50),
        // Device y = 25.3: row 25.
        (
            Affine {
                a: 0.5,
                b: 0.0,
                c: 0.0,
                d: 0.5,
                e: 0.0,
                f: 0.0,
            },
            25,
        ),
        // Device y = 20.24 + 10 = 30.24: row 30.
        (
            Affine {
                a: 0.4,
                b: 0.0,
                c: 0.0,
                d: 0.4,
                e: 0.0,
                f: 10.0,
            },
            30,
        ),
    ] {
        let bytes = rendered(&mut device, &scene, transform);
        let x = (32.0 * transform.a + transform.e) as u32;
        assert_eq!(
            inked_row(&bytes, x),
            row,
            "the mark follows the viewport, transform {transform:?}"
        );
    }
}

/// A quarter turn exchanges the axes and the mark follows: the run of pixels is a
/// device *column*, found in device space where the snap is stated.
#[test]
fn a_quarter_turn_marks_a_device_column() {
    let mut device = device();
    let scene = filled(&mut device, &zero_height_rectangle(50.3));
    // (x, y) → (y, x): maps the horizontal rule at y ≈ 50.3 onto device column 50,
    // rows 10 to 54.
    let bytes = rendered(
        &mut device,
        &scene,
        Affine {
            a: 0.0,
            b: 1.0,
            c: 1.0,
            d: 0.0,
            e: 0.0,
            f: 0.0,
        },
    );
    let columns: Vec<u32> = (0..SIZE).filter(|&x| alpha(&bytes, x, 32) > 0).collect();
    assert_eq!(columns, vec![50], "one whole device column");
    assert_eq!(alpha(&bytes, 50, 32), 255);
    assert_eq!(
        alpha(&bytes, 50, 8),
        0,
        "the mark keeps the subpath's extent"
    );
}

/// Under a shear no device axis is an outline axis, so the band stays — one device
/// pixel about the subpath's own line, antialiased rather than snapped. The mark must
/// still not disappear, which is the clause's whole point; where exactly its
/// half-covered edge pixels fall is the band's own business.
#[test]
fn a_shear_keeps_the_band_and_the_mark_does_not_disappear() {
    let mut device = device();
    let scene = filled(&mut device, &zero_height_rectangle(50.3));
    let bytes = rendered(
        &mut device,
        &scene,
        Affine {
            a: 1.0,
            b: 0.25,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        },
    );
    // At x = 32 the sheared line passes device y = 50.3 + 32·0.25 = 58.3.
    let inked: Vec<u32> = (0..SIZE).filter(|&y| alpha(&bytes, 32, y) > 0).collect();
    assert!(!inked.is_empty(), "the mark shall not disappear (§10.7.4)");
    assert!(
        inked.iter().all(|&y| (57..=60).contains(&y)),
        "ink stays within the band about the line, found rows {inked:?}"
    );
}

/// A shape *thinner* than a pixel is not this rule's business: it has an area, and
/// what an area gets is the coverage it implies. Only a subpath with literally no
/// extent vanishes at every placement — exact equality is the collapse test, as in
/// the caller's `Extent::collapse`.
#[test]
fn a_sliver_with_area_gets_coverage_not_a_mark() {
    let mut device = device();
    let sliver = [
        Segment::MoveTo(Point::new(10.0, 50.0)),
        Segment::LineTo(Point::new(54.0, 50.0)),
        Segment::LineTo(Point::new(54.0, 50.1)),
        Segment::LineTo(Point::new(10.0, 50.1)),
        Segment::Close,
    ];
    let scene = filled(&mut device, &sliver);
    let bytes = rendered(&mut device, &scene, Affine::IDENTITY);
    let a = alpha(&bytes, 32, 50);
    assert!(
        a > 0 && a < 128,
        "a tenth of a pixel's coverage, not a whole pixel's: alpha {a}"
    );
}

/// A point is §8.5.3.3.1's case rather than this one — there is no axis to lay a
/// mark along, and the table does not claim it.
#[test]
fn a_single_point_subpath_makes_no_mark() {
    let mut device = device();
    let dot = [Segment::MoveTo(Point::new(32.0, 32.0)), Segment::Close];
    let scene = filled(&mut device, &dot);
    let bytes = rendered(&mut device, &scene, Affine::IDENTITY);
    assert!(
        (0..SIZE).all(|y| (0..SIZE).all(|x| alpha(&bytes, x, y) == 0)),
        "no ink anywhere"
    );
}

/// The two halves of a mixed path go their separate ways: the subpaths with area fill
/// under the command's own rule, the collapsed one marks its row — one command, both
/// answers, which is what lets the caller upload the *original* path and keep its
/// cache identity.
#[test]
fn a_mixed_path_fills_its_area_and_marks_its_ruling() {
    let mut device = device();
    let mixed = [
        Segment::MoveTo(Point::new(10.0, 10.0)),
        Segment::LineTo(Point::new(30.0, 10.0)),
        Segment::LineTo(Point::new(30.0, 30.0)),
        Segment::LineTo(Point::new(10.0, 30.0)),
        Segment::Close,
        Segment::MoveTo(Point::new(10.0, 50.3)),
        Segment::LineTo(Point::new(54.0, 50.3)),
        Segment::LineTo(Point::new(54.0, 50.3)),
        Segment::LineTo(Point::new(10.0, 50.3)),
        Segment::Close,
    ];
    let scene = filled(&mut device, &mixed);
    let bytes = rendered(&mut device, &scene, Affine::IDENTITY);
    assert_eq!(alpha(&bytes, 20, 20), 255, "the square fills");
    assert_eq!(alpha(&bytes, 32, 50), 255, "the ruling marks its row");
    assert_eq!(alpha(&bytes, 32, 45), 0, "and nothing between them");
}

//! Record replay (ADR 0087): the walk paid once, replayed per viewport — and held to
//! byte identity against the walk it replaces.
//!
//! ADR 0084 stage A. The claim under test is sharp: a retained scene rendered at a
//! *new* viewport from its records produces **the bytes the full walk would have
//! produced**, because the records carry only the walk's per-scene answers and every
//! per-viewport answer — compose, cull, seat, instance bytes — is computed fresh by
//! the same arithmetic. Anything weaker would make the replay a second implementation
//! of the encode, which is exactly what it must not be.

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

use quorra_gpu::{Coverage, Device, EncodeSource, Options, RetainedScene, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, GroupSpec, Paint, Point, Scene, SceneBuilder,
    Segment, Stroke,
};

const SIZE: u32 = 96;

fn device() -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        coverage: Coverage::Compute,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

fn pixels(device: &mut Device, scene: &Scene, viewport: &Viewport<'_>) -> Vec<u8> {
    device
        .render(scene, viewport, Target::Readback)
        .expect("renders")
        .into_raster()
        .unwrap()
        .into_pixels()
}

/// A page with every replayable shape on it: plain fills (compute tiles), an
/// axis-aligned rectangle fill (the analytic route), a collapsed ruling (§10.7.4
/// marks), a clipped fill, and a stroke (a Slow record).
#[allow(clippy::too_many_lines)] // one fixture, five shapes, read top to bottom
fn page(device: &mut Device) -> Scene {
    let mut builder = SceneBuilder::new();
    let triangle = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(8.0, 8.0)),
            Segment::LineTo(Point::new(56.0, 20.0)),
            Segment::LineTo(Point::new(24.0, 60.0)),
            Segment::Close,
        ])
        .unwrap();
    let square = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(50.0, 50.0)),
            Segment::LineTo(Point::new(80.0, 50.0)),
            Segment::LineTo(Point::new(80.0, 78.0)),
            Segment::LineTo(Point::new(50.0, 78.0)),
            Segment::Close,
        ])
        .unwrap();
    let ruling = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(10.0, 70.5)),
            Segment::LineTo(Point::new(88.0, 70.5)),
            Segment::LineTo(Point::new(88.0, 70.5)),
            Segment::LineTo(Point::new(10.0, 70.5)),
            Segment::Close,
        ])
        .unwrap();
    let wave = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(12.0, 86.0)),
            Segment::CubicTo {
                c1: Point::new(30.0, 74.0),
                c2: Point::new(60.0, 96.0),
                to: Point::new(86.0, 82.0),
            },
        ])
        .unwrap();
    let clip = builder
        .clip(square, Affine::IDENTITY, FillRule::NonZero, None)
        .unwrap();
    builder
        .fill(
            triangle,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Solid(Color::new(0.8, 0.2, 0.1, 1.0)),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    builder
        .fill(
            square,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Solid(Color::new(0.1, 0.4, 0.8, 0.8)),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    builder
        .fill(
            triangle,
            Affine {
                a: 0.6,
                b: 0.1,
                c: -0.1,
                d: 0.6,
                e: 40.0,
                f: 6.0,
            },
            FillRule::EvenOdd,
            Paint::Solid(Color::new(0.1, 0.6, 0.2, 1.0)),
            Some(clip),
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    builder
        .fill(
            ruling,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0)),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    builder
        .stroke(
            wave,
            Affine::IDENTITY,
            Stroke {
                width: 2.5,
                adjust: false,
                cap: quorra_scene::LineCap::Round,
                join: quorra_scene::LineJoin::Round,
                miter_limit: 4.0,
            },
            Paint::Solid(Color::new(0.3, 0.1, 0.6, 1.0)),
            None,
            BlendMode::Normal,
            None,
        )
        .unwrap();
    builder.finish()
}

/// The three viewports of a zoom gesture: none of them the one the records were made
/// at, and the middle one oblique so the analytic route falls back per viewport.
fn viewports() -> [Affine; 3] {
    [
        Affine {
            a: 1.4,
            b: 0.0,
            c: 0.0,
            d: 1.4,
            e: -8.0,
            f: -4.0,
        },
        Affine {
            a: 1.1,
            b: 0.2,
            c: -0.2,
            d: 1.1,
            e: 6.0,
            f: 2.0,
        },
        Affine {
            a: 0.7,
            b: 0.0,
            c: 0.0,
            d: 0.7,
            e: 12.0,
            f: 10.0,
        },
    ]
}

/// The claim itself: a zoom step is record-replayed, and its bytes are the full
/// walk's bytes — for every shape the admission allows, including the Slow stroke.
#[test]
fn a_record_replayed_frame_is_byte_identical_to_the_walk() {
    let mut device = device();
    device.wait_until_warm();
    let scene = page(&mut device);
    let mut retained = RetainedScene::new(scene.clone());
    // First frame at the identity walks and records.
    let first = device
        .render_retained(
            &mut retained,
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders");
    assert_eq!(first.encode_source(), EncodeSource::Encoded);
    for (step, transform) in viewports().into_iter().enumerate() {
        let viewport = Viewport::full(SIZE, SIZE, transform);
        let replayed = device
            .render_retained(&mut retained, &viewport, Target::Readback)
            .expect("renders");
        assert_eq!(
            replayed.encode_source(),
            EncodeSource::RecordReplayed,
            "step {step}: a new viewport over unchanged everything-else replays"
        );
        let replayed = replayed.into_raster().unwrap().into_pixels();
        let walked = pixels(&mut device, &scene, &viewport);
        assert_eq!(
            replayed, walked,
            "step {step}: the replay is the walk's bytes or it is a second implementation"
        );
    }
}

/// The identical viewport still takes the cheaper road: a full replay of the retained
/// encode, not a record replay.
#[test]
fn the_same_viewport_replays_the_encode_not_the_records() {
    let mut device = device();
    let scene = page(&mut device);
    let mut retained = RetainedScene::new(scene);
    let viewport = Viewport::full(SIZE, SIZE, Affine::IDENTITY);
    device
        .render_retained(&mut retained, &viewport, Target::Readback)
        .expect("renders");
    let again = device
        .render_retained(&mut retained, &viewport, Target::Readback)
        .expect("renders");
    assert_eq!(again.encode_source(), EncodeSource::Replayed);
}

/// Admission: a frame with a child layer re-walks at a new viewport, exactly as
/// before the records existed. (A group is the cheapest unreplayable structure to
/// state; masks, residues, atlas and winding tiles die by the same rule at their own
/// sites.)
#[test]
fn a_frame_with_a_child_layer_re_encodes_instead_of_replaying() {
    let mut device = device();
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(10.0, 10.0)),
            Segment::LineTo(Point::new(40.0, 10.0)),
            Segment::LineTo(Point::new(40.0, 40.0)),
            Segment::Close,
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    builder
        .group(
            GroupSpec {
                isolated: true,
                knockout: false,
                alpha: 0.5,
                blend: BlendMode::Normal,
                compose: Compose::SrcOver,
                clip: None,
                mask: None,
            },
            |builder| {
                builder.fill(
                    outline,
                    Affine::IDENTITY,
                    FillRule::NonZero,
                    Paint::Solid(Color::new(0.5, 0.2, 0.7, 1.0)),
                    None,
                    BlendMode::Normal,
                    Compose::SrcOver,
                    None,
                )
            },
        )
        .unwrap();
    let mut retained = RetainedScene::new(builder.finish());
    device
        .render_retained(
            &mut retained,
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders");
    let zoomed = device
        .render_retained(
            &mut retained,
            &Viewport::full(
                SIZE,
                SIZE,
                Affine {
                    a: 1.3,
                    b: 0.0,
                    c: 0.0,
                    d: 1.3,
                    e: 0.0,
                    f: 0.0,
                },
            ),
            Target::Readback,
        )
        .expect("renders");
    assert_eq!(zoomed.encode_source(), EncodeSource::Encoded);
}

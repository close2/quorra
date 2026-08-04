//! Command culling: what a frame skips must be exactly what it could not have drawn.
//!
//! ADR 0015. The encoder rejects a command whose device bounds reach no pixel of the
//! target, which is what makes a zoomed frame cost what it shows rather than what the
//! page holds. Every test here is one half of that claim:
//!
//! - **Nothing that could draw is dropped.** A command reaching in from outside the
//!   target — a stroke whose width carries it in, a fill straddling the edge — marks
//!   the pixels it would have marked, byte for byte.
//! - **Everything dropped is reported.** `Counters::commands_culled` counts what was
//!   skipped, so the saving is measured rather than assumed, and a scene drawn whole
//!   reports zero.
//! - **Visibility does not decide validity.** A command referring to a resource that
//!   does not exist refuses wherever it lands: a refusal that depended on the
//!   viewport would be a worse defect than the work culling saves.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Counters, Device, Options, RenderError, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, LineCap, LineJoin, Paint, Point, RampId, Rect,
    Scene, SceneBuilder, Segment, ShadingKind, Stroke,
};

/// The software adapter, as everywhere in this suite: deterministic, always present.
fn device() -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

const SIZE: u32 = 32;

fn render(device: &mut Device, scene: &Scene) -> (Vec<u8>, Counters) {
    let frame = device
        .render(
            scene,
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders");
    let counters = frame.counters();
    (frame.into_raster().unwrap().into_pixels(), counters)
}

fn alpha_at(pixels: &[u8], x: u32, y: u32) -> u8 {
    pixels[((y * SIZE + x) * 4 + 3) as usize]
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

fn black() -> Paint {
    Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0))
}

/// One rectangle inside the target, plus commands of every lane placed far outside
/// it: the frame must be the one the visible rectangle alone produces, and the count
/// of skipped commands must be exactly the number placed outside.
#[test]
fn commands_off_the_target_are_counted_and_change_no_pixel() {
    let mut device = device();
    let seen = Rect::new(Point::new(8.0, 8.0), Point::new(24.0, 24.0));
    let outline = device.upload_outline(&rect_outline(seen)).unwrap();

    let mut only_visible = SceneBuilder::new();
    only_visible
        .rect(
            seen,
            Affine::IDENTITY,
            Color::new(0.0, 0.0, 0.0, 1.0),
            None,
            None,
        )
        .unwrap();
    let (want, plain) = render(&mut device, &only_visible.finish());
    assert_eq!(
        plain.commands_culled, 0,
        "a scene drawn whole culls nothing"
    );

    // The same rectangle, plus a rect, a fill and a stroke a long way off the target
    // — far enough that no margin could reach back — and one of each again just past
    // the edge, where the arithmetic is delicate rather than obvious.
    let mut with_absent = SceneBuilder::new();
    with_absent
        .rect(
            seen,
            Affine::IDENTITY,
            Color::new(0.0, 0.0, 0.0, 1.0),
            None,
            None,
        )
        .unwrap();
    let elsewhere = [
        Affine::translate(-4_000.0, 0.0),
        Affine::translate(4_000.0, 0.0),
        Affine::translate(0.0, -4_000.0),
        Affine::translate(0.0, SIZE as f32 + 40.0),
    ];
    for placement in elsewhere {
        with_absent
            .rect(seen, placement, Color::new(0.0, 0.0, 0.0, 1.0), None, None)
            .unwrap();
        with_absent
            .fill(
                outline,
                placement,
                FillRule::NonZero,
                black(),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
        with_absent
            .stroke(
                outline,
                placement,
                Stroke {
                    width: 2.0,
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
    }
    let (got, counters) = render(&mut device, &with_absent.finish());

    assert_eq!(
        counters.commands_culled,
        u32::try_from(elsewhere.len()).unwrap() * 3,
        "every command placed off the target is counted once"
    );
    assert_eq!(
        got, want,
        "commands that cannot reach the target change no pixel"
    );
}

/// A stroke whose *outline* lies outside the target but whose width carries it back
/// in must draw. The bound a cull tests has to grow by the stroke's own reach, which
/// is the one place where an outline's hull is not what marks pixels.
#[test]
fn a_stroke_reaching_in_from_outside_still_draws() {
    let mut device = device();
    // A vertical segment three pixels left of the target, stroked ten wide: it covers
    // x ∈ [−8, 2], so device columns 0 and 1 are fully inside it.
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(-3.0, 6.0)),
            Segment::LineTo(Point::new(-3.0, 26.0)),
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    builder
        .stroke(
            outline,
            Affine::IDENTITY,
            Stroke {
                width: 10.0,
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
    let (pixels, counters) = render(&mut device, &builder.finish());

    assert_eq!(
        counters.commands_culled, 0,
        "a stroke whose width reaches the target is not culled"
    );
    assert_eq!(
        alpha_at(&pixels, 0, 16),
        255,
        "column 0 lies inside the stroked band"
    );
    assert_eq!(
        alpha_at(&pixels, 1, 16),
        255,
        "column 1 lies inside the stroked band"
    );
    assert_eq!(
        alpha_at(&pixels, 2, 16),
        0,
        "the band ends at x = 2, so column 2 is untouched"
    );
}

/// A fill straddling the target's edge keeps its partial coverage. The expected byte
/// is derivable rather than observed: the shape covers half of column 0, coverage
/// quantises once as `round(0.5 × 255)` (ADR 0005/0008), and a black fill on
/// transparency reads back with that as its alpha.
#[test]
fn a_fill_straddling_the_edge_keeps_its_coverage() {
    let mut device = device();
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(-5.0, 6.0),
            Point::new(0.5, 26.0),
        )))
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
    let (pixels, counters) = render(&mut device, &builder.finish());

    assert_eq!(counters.commands_culled, 0, "the fill reaches column 0");
    assert_eq!(
        alpha_at(&pixels, 0, 16),
        128,
        "half of column 0 is covered: round(0.5 × 255)"
    );
    assert_eq!(alpha_at(&pixels, 1, 16), 0, "the fill ends inside column 0");
}

/// Inside the target but outside its clip is just as invisible, and counted the same
/// way: the test is bounds ∩ clip ∩ target, not bounds ∩ target.
#[test]
fn a_command_clipped_away_is_culled_too() {
    let mut device = device();
    let clip_outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(20.0, 20.0),
            Point::new(30.0, 30.0),
        )))
        .unwrap();
    let mut builder = SceneBuilder::new();
    let clip = builder
        .clip(clip_outline, Affine::IDENTITY, FillRule::NonZero, None)
        .unwrap();
    builder
        .rect(
            Rect::new(Point::new(2.0, 2.0), Point::new(10.0, 10.0)),
            Affine::IDENTITY,
            Color::new(0.0, 0.0, 0.0, 1.0),
            Some(clip),
            None,
        )
        .unwrap();
    let (pixels, counters) = render(&mut device, &builder.finish());

    assert_eq!(counters.commands_culled, 1);
    assert!(
        pixels.iter().all(|&byte| byte == 0),
        "a command clipped away marks nothing"
    );
}

/// **Visibility does not decide validity.** A fill naming a ramp that was never
/// uploaded refuses by name whether it lands on the target or a mile off it —
/// otherwise a scene's validity would depend on where the viewer happened to be
/// looking, and a caller could not trust a frame that drew.
#[test]
fn an_unknown_ramp_refuses_even_out_of_sight() {
    let mut device = device();
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(8.0, 8.0),
        )))
        .unwrap();
    let shading = Paint::Shading {
        ramp: RampId(4_242),
        transform: Affine::IDENTITY,
        kind: ShadingKind::Axial {
            start: Point::new(0.0, 0.0),
            end: Point::new(8.0, 0.0),
            extend: (true, true),
        },
    };
    for placement in [Affine::IDENTITY, Affine::translate(-4_000.0, 0.0)] {
        let mut builder = SceneBuilder::new();
        builder
            .fill(
                outline,
                placement,
                FillRule::NonZero,
                shading,
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
        let refused = device.render(
            &builder.finish(),
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        );
        assert!(
            matches!(refused, Err(RenderError::UnknownRamp { .. })),
            "an unknown ramp refuses at {placement:?}, got {refused:?}"
        );
    }
}

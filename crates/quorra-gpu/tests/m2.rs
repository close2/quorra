//! The M2 harness: the upload/reference round trip, the resource budget as a stated
//! refusal, and the loud not-yet-drawable boundary.
//!
//! What M2 can and cannot prove, stated rather than implied: uploads, identity,
//! budgets and refusals are provable now; the *drawable* half of the round trip — the
//! 107 outlines actually painting 5 933 fills — arrives with the lanes (M4/M5), and
//! its test lands there.

// Same test-file lint policy as m1.rs (clippy.toml's allowances stop at #[cfg(test)]).
// The casts build bounded scene coordinates (indices < 5 933), exact in f32.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss
)]

use quorra_gpu::{Device, DeviceError, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, Paint, Point, Rect, ResourceId, SceneBuilder,
    Segment,
};

fn device() -> Device {
    // Any Vulkan adapter will do; llvmpipe exists everywhere the suite runs.
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

fn glyph_like_outline() -> Vec<Segment> {
    vec![
        Segment::MoveTo(Point::new(0.0, 0.0)),
        Segment::LineTo(Point::new(4.0, 0.0)),
        Segment::CubicTo {
            c1: Point::new(5.0, 1.0),
            c2: Point::new(5.0, 5.0),
            to: Point::new(4.0, 6.0),
        },
        Segment::Close,
    ]
}

/// §2.2's shape at device level: 107 distinct outlines uploaded once, thousands of
/// references built against them with no further device involvement, and rendering a
/// *rect-only* scene afterwards touches none of it (a zoom re-uploads nothing).
#[test]
fn the_upload_once_reference_many_round_trip() {
    let mut device = device();
    let outline = glyph_like_outline();
    let ids: Vec<_> = (0..107)
        .map(|_| device.upload_outline(&outline).expect("within budget"))
        .collect();
    let resident_after_uploads = device.resource_bytes_in_use();
    assert!(resident_after_uploads > 0);

    // A scene referencing every outline many times over: built with no device access,
    // exactly as the caller's worker thread would.
    let mut builder = SceneBuilder::new();
    for i in 0..5_933_usize {
        builder
            .fill(
                ids[i % ids.len()],
                Affine::translate(i as f32 % 80.0, (i / 80) as f32),
                FillRule::NonZero,
                Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0)),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .expect("valid fill");
    }
    let scene = builder.finish();
    assert_eq!(scene.cost().commands, 5_933);

    // Since M4 the fills draw through the glyph lane, and the keying proves §2.2's
    // arithmetic: 5 933 fills, integer translations (phase 0), one linear part —
    // exactly 107 distinct keys, one atlas entry per distinct outline.
    let frame = device
        .render(
            &scene,
            &Viewport::full(1200, 80, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("fills draw since M4");
    assert_eq!(frame.counters().atlas_distinct_keys, 107);
    assert_eq!(frame.counters().atlas_entries, 107);
    assert_eq!(frame.counters().distinct_outlines, 107);
    // A second frame re-rasterises nothing: same keys, all tiles hit.
    let frame = device
        .render(
            &scene,
            &Viewport::full(1200, 80, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("second frame");
    assert_eq!(frame.counters().atlas_entries, 107);

    // Rendering rect-only content afterwards neither needs nor touches the residency.
    let mut rects = SceneBuilder::new();
    rects
        .rect(
            Rect::new(Point::new(1.0, 1.0), Point::new(9.0, 9.0)),
            Affine::IDENTITY,
            Color::new(0.0, 0.5, 1.0, 1.0),
            None,
            None,
        )
        .expect("valid rect");
    let rect_scene = rects.finish();
    for _ in 0..2 {
        device
            .render(
                &rect_scene,
                &Viewport::full(16, 16, Affine::IDENTITY),
                Target::Readback,
            )
            .expect("rect lane exists since M1");
    }
    assert_eq!(
        device.resource_bytes_in_use(),
        resident_after_uploads,
        "rendering must not consume or duplicate resident resources"
    );

    for id in ids {
        device.release(id).expect("resident resources release");
    }
    assert_eq!(device.resource_bytes_in_use(), 0);
}

/// The resource budget is discoverable before any upload ([`Device::limits`]), and
/// hitting it is a refusal naming all three numbers (§5).
#[test]
fn resource_budget_is_discoverable_and_loud() {
    // Big enough for exactly one of the test outlines, not two — *measured* rather
    // than written down, because what an outline costs is a property of what the
    // device keeps for it, and a constant here would fail the day that changes for a
    // good reason. It changed twice: ADR 0016 added the quadratics the GPU lane draws,
    // and ADR 0075 moved them off the upload, so what this measures now is the
    // segments alone until a frame takes that lane.
    let budget = {
        let mut sizing = Device::headless(&Options {
            adapter: Some("llvmpipe".into()),
            ..Options::default()
        })
        .expect("llvmpipe is present");
        sizing
            .upload_outline(&glyph_like_outline())
            .expect("the default budget holds one outline");
        sizing.resource_bytes_in_use() * 3 / 2
    };
    let mut device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        max_resource_bytes: budget,
        ..Options::default()
    })
    .expect("llvmpipe is present");
    assert_eq!(device.limits().max_resource_bytes, budget);

    let first = device
        .upload_outline(&glyph_like_outline())
        .expect("one outline fits the budget");
    match device.upload_outline(&glyph_like_outline()) {
        Err(DeviceError::ResourceBudgetExceeded {
            needed,
            in_use,
            budget: named,
        }) => {
            assert_eq!(named, budget);
            assert!(in_use > 0);
            assert!(needed > budget);
        }
        other => panic!("expected ResourceBudgetExceeded, got {other:?}"),
    }

    // Releasing makes room again: the budget is about residency, not history.
    device.release(first).expect("release");
    device
        .upload_outline(&glyph_like_outline())
        .expect("freed bytes are reusable");
}

/// What is drawable grew with each milestone: strokes render since M5, groups
/// composite since M6, and M7 closed the list — every scene command draws.
#[test]
fn each_missing_lane_is_named() {
    let mut device = device();
    let outline = device
        .upload_outline(&glyph_like_outline())
        .expect("upload");
    let viewport = Viewport::full(8, 8, Affine::IDENTITY);

    let mut stroked = SceneBuilder::new();
    stroked
        .stroke(
            outline,
            Affine::IDENTITY,
            quorra_scene::Stroke {
                width: 1.0,
                cap: quorra_scene::LineCap::Butt,
                join: quorra_scene::LineJoin::Miter,
                miter_limit: 4.0,
            },
            Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0)),
            None,
            BlendMode::Normal,
            None,
        )
        .expect("valid stroke");
    device
        .render(&stroked.finish(), &viewport, Target::Readback)
        .expect("strokes draw since M5");

    let mut grouped = SceneBuilder::new();
    grouped
        .group(
            quorra_scene::GroupSpec {
                alpha: 0.5,
                blend: BlendMode::Multiply,
                clip: None,
                knockout: false,
                mask: None,
                isolated: true,
                compose: Compose::SrcOver,
            },
            |b| {
                b.rect(
                    Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
                    Affine::IDENTITY,
                    Color::new(0.0, 0.0, 0.0, 1.0),
                    None,
                    None,
                )
            },
        )
        .expect("valid group");
    device
        .render(&grouped.finish(), &viewport, Target::Readback)
        .expect("groups composite since M6");
}

/// A released id stays dead: re-releasing it is the double-release error, and ids are
/// never reused across resource families.
#[test]
fn released_ids_stay_dead() {
    let mut device = device();
    let id = device
        .upload_outline(&glyph_like_outline())
        .expect("upload");
    device.release(id).expect("first release");
    assert!(matches!(
        device.release(id),
        Err(DeviceError::UnknownResource {
            id: ResourceId::Outline(_)
        })
    ));
}

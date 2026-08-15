//! The M4/M5 harness: the glyph and path lanes, held against each other and against
//! the lanes that already passed their gates.
//!
//! The strongest checks here are **cross-lane**: the same geometry drawn through two
//! independent code paths (the analytic rectangle shader of ADR 0005 versus the CPU
//! rasteriser of ADR 0008; the atlas path versus the scratch fallback) must agree —
//! exactly where both are exact, and within one unorm step where the coverage byte's
//! single quantisation is the stated difference.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::arithmetic_side_effects,
    clippy::needless_pass_by_value,
    clippy::too_many_lines
)]

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, LineCap, LineJoin, Paint, Point, Rect,
    SceneBuilder, Segment, Stroke,
};

mod common;

use common::headless::render;
use common::scene::rect_outline;

fn device_with(options: Options) -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..options
    })
    .expect("llvmpipe is present wherever this suite runs")
}

fn device() -> Device {
    device_with(Options::default())
}

/// The same four edges as [`common::scene::rect_outline`], with one redundant vertex
/// halfway along the top: the identical region, and **not** what
/// `quorra_scene::axis_aligned_rect` recognises, so a solid fill of it still rasterises
/// its own coverage.
///
/// Since ADR 0047 a solid fill of a *recognised* rectangle takes the analytic lane, and
/// this is what is left to put a rectangle through the atlas and the scratch sheet —
/// which is what the three tests below are about. Written here rather than reached for
/// per test, because the alternative is three tests that pass while comparing one lane
/// with itself.
fn rasterised_rect_outline(rect: Rect) -> Vec<Segment> {
    vec![
        Segment::MoveTo(rect.min),
        // On the top edge by construction: same y, an x strictly between the corners.
        Segment::LineTo(Point::new((rect.min.x + rect.max.x) * 0.5, rect.min.y)),
        Segment::LineTo(Point::new(rect.max.x, rect.min.y)),
        Segment::LineTo(rect.max),
        Segment::LineTo(Point::new(rect.min.x, rect.max.y)),
        Segment::Close,
    ]
}

fn max_diff(a: &[u8], b: &[u8]) -> i32 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| (i32::from(*x) - i32::from(*y)).abs())
        .max()
        .unwrap_or(0)
}

/// Largest per-channel difference after re-premultiplying both straight-alpha
/// rasters — undoing the readback's demultiply amplification so a bound on the
/// stored (premultiplied) values can be asserted directly.
fn max_diff_premultiplied(a: &[u8], b: &[u8]) -> i32 {
    assert_eq!(a.len(), b.len());
    let premul = |pixel: &[u8]| {
        let alpha = u32::from(pixel[3]);
        [
            ((u32::from(pixel[0]) * alpha + 127) / 255) as i32,
            ((u32::from(pixel[1]) * alpha + 127) / 255) as i32,
            ((u32::from(pixel[2]) * alpha + 127) / 255) as i32,
            alpha as i32,
        ]
    };
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(pa, pb)| {
            let (qa, qb) = (premul(pa), premul(pb));
            (0..4).map(|i| (qa[i] - qb[i]).abs()).max().unwrap_or(0)
        })
        .max()
        .unwrap_or(0)
}

/// The same fractional rectangle through the analytic rect lane and, as an outline,
/// through the glyph lane: interior and exterior agree exactly; edge pixels agree
/// within one unorm step (the coverage byte is quantised once in the cached tile —
/// the stated difference of ADR 0008, and the whole difference).
///
/// The fill's outline carries a redundant vertex so that it *stays* on the glyph lane
/// (ADR 0047). Without it both sides would be the analytic lane and the comparison
/// would be between a computation and itself.
///
/// The comparison happens in premultiplied space: the straight-alpha readback
/// re-amplifies a one-step premultiplied difference by 255/α (the demultiply rule,
/// ADR 0005), and edge pixels have partial α by construction — so the bound is
/// stated where the arithmetic actually lives.
#[test]
fn rect_lane_and_glyph_lane_agree_on_a_rectangle() {
    let shape = Rect::new(Point::new(2.25, 3.5), Point::new(11.75, 13.125));
    let color = Color::new(0.8, 0.3, 0.1, 1.0);

    let mut device = device();
    let mut via_rect = SceneBuilder::new();
    via_rect
        .rect(shape, Affine::IDENTITY, color, None, None)
        .expect("valid rect");
    let rect_pixels = render(&mut device, &via_rect.finish(), 16, 16);

    let outline = device
        .upload_outline(&rasterised_rect_outline(shape))
        .expect("upload");
    let mut via_fill = SceneBuilder::new();
    via_fill
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Solid(color),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("valid fill");
    let fill_pixels = render(&mut device, &via_fill.finish(), 16, 16);

    let diff = max_diff_premultiplied(&rect_pixels, &fill_pixels);
    assert!(
        diff <= 1,
        "coverage implementations differ by {diff} premultiplied steps (bound 1)"
    );
    // Interior exact: full coverage has no quantisation to differ by.
    let interior = (8 * 16 + 8) * 4;
    assert_eq!(
        rect_pixels[interior..interior + 4],
        fill_pixels[interior..interior + 4]
    );
}

/// The atlas path and the scratch fallback produce byte-identical frames: one
/// rasteriser feeds both, so starving the atlas (budget too small for any tile)
/// changes the route and not one pixel.
///
/// The shape is a rectangle with a redundant vertex, which is what keeps it on the two
/// lanes this compares rather than on the analytic one (ADR 0047).
#[test]
fn atlas_and_scratch_fallback_are_byte_identical() {
    let glyph = Rect::new(Point::new(0.25, 0.5), Point::new(6.75, 8.5));
    let color = Color::new(0.1, 0.1, 0.1, 1.0);

    let scene_for = |device: &mut Device| {
        let outline = device
            .upload_outline(&rasterised_rect_outline(glyph))
            .expect("upload");
        let mut builder = SceneBuilder::new();
        for i in 0..64_u32 {
            builder
                .fill(
                    outline,
                    Affine::translate((i % 8) as f32 * 8.0, (i / 8) as f32 * 10.0),
                    FillRule::NonZero,
                    Paint::Solid(color),
                    None,
                    BlendMode::Normal,
                    Compose::SrcOver,
                    None,
                )
                .expect("valid fill");
        }
        builder.finish()
    };

    let mut cached = device();
    let scene = scene_for(&mut cached);
    let with_atlas = render(&mut cached, &scene, 80, 96);

    let mut starved = device_with(Options {
        atlas_budget: 1, // a 1×1 atlas: nothing fits, everything falls through
        ..Options::default()
    });
    let scene = scene_for(&mut starved);
    let without_atlas = render(&mut starved, &scene, 80, 96);

    assert_eq!(with_atlas, without_atlas);
}

/// §4.5's fifth decision, observable: with the quantum off, every distinct sub-pixel
/// phase is its own key; with a quantum of 1/4, the same hundred phases collapse to
/// four keys. The counter is the count of distinct keys, per §6.3 — never a hit rate.
///
/// The shape has to be one the atlas is asked about at all, so it is a rectangle with a
/// redundant vertex (ADR 0047): a recognised one would take the analytic lane and this
/// test would read zero keys under both settings.
#[test]
fn the_quantum_is_settable_and_off_is_exact() {
    let glyph = Rect::new(Point::new(0.0, 0.0), Point::new(4.0, 4.0));
    let color = Color::new(0.0, 0.0, 0.0, 1.0);
    let scene_for = |device: &mut Device| {
        let outline = device
            .upload_outline(&rasterised_rect_outline(glyph))
            .expect("upload");
        let mut builder = SceneBuilder::new();
        for i in 0..100_u32 {
            builder
                .fill(
                    outline,
                    Affine::translate(i as f32 * 6.01, 0.0), // 100 distinct phases
                    FillRule::NonZero,
                    Paint::Solid(color),
                    None,
                    BlendMode::Normal,
                    Compose::SrcOver,
                    None,
                )
                .expect("valid fill");
        }
        builder.finish()
    };

    let mut exact = device_with(Options {
        glyph_quantum: None,
        ..Options::default()
    });
    let scene = scene_for(&mut exact);
    let frame = exact
        .render(
            &scene,
            &Viewport::full(640, 8, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders");
    assert_eq!(
        frame.counters().atlas_distinct_keys,
        100,
        "with the quantum off, an arbitrary phase never aliases"
    );

    let mut quantised = device_with(Options {
        glyph_quantum: Some(4),
        ..Options::default()
    });
    let scene = scene_for(&mut quantised);
    let frame = quantised
        .render(
            &scene,
            &Viewport::full(640, 8, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders");
    assert!(
        frame.counters().atlas_distinct_keys <= 5,
        "a 1/4 quantum folds 100 phases into at most 4 keys (5 with the wrap), got {}",
        frame.counters().atlas_distinct_keys
    );
}

/// §4.7's discriminating fill-rule case, end to end on the device: a nested subpath
/// wound the same way fills under non-zero and holes under even-odd.
#[test]
fn fill_rules_differ_on_nested_same_winding_through_the_device() {
    let mut path = rect_outline(Rect::new(Point::new(10.0, 10.0), Point::new(150.0, 150.0)));
    path.extend(rect_outline(Rect::new(
        Point::new(50.0, 50.0),
        Point::new(110.0, 110.0),
    )));

    let mut device = device();
    let outline = device.upload_outline(&path).expect("upload");
    let render_rule = |device: &mut Device, rule: FillRule| {
        let mut builder = SceneBuilder::new();
        builder
            .fill(
                outline,
                Affine::IDENTITY,
                rule,
                Paint::Solid(Color::new(0.0, 0.6, 0.0, 1.0)),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .expect("valid fill");
        render(device, &builder.finish(), 160, 160)
    };
    let nonzero = render_rule(&mut device, FillRule::NonZero);
    let evenodd = render_rule(&mut device, FillRule::EvenOdd);

    let pixel = |data: &[u8], x: usize, y: usize| data[(y * 160 + x) * 4 + 1];
    // Centre of the nested subpath: filled under non-zero, holed under even-odd.
    assert_eq!(pixel(&nonzero, 80, 80), 153);
    assert_eq!(pixel(&evenodd, 80, 80), 0);
    // Between the two boundaries: filled under both.
    assert_eq!(pixel(&nonzero, 30, 30), 153);
    assert_eq!(pixel(&evenodd, 30, 30), 153);
}

/// A stroked horizontal line with butt caps is a rectangle: the stroke expansion,
/// the path lane and the analytic rect lane agree within the one quantisation step.
#[test]
fn stroke_agrees_with_the_rectangle_it_is() {
    let mut device = device();
    let line = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(20.0, 100.0)),
            Segment::LineTo(Point::new(180.0, 100.0)),
        ])
        .expect("upload");
    let mut stroked = SceneBuilder::new();
    stroked
        .stroke(
            line,
            Affine::IDENTITY,
            Stroke {
                width: 6.0,
                cap: LineCap::Butt,
                join: LineJoin::Miter,
                miter_limit: 10.0,
            },
            Paint::Solid(Color::new(0.2, 0.2, 0.9, 1.0)),
            None,
            BlendMode::Normal,
            None,
        )
        .expect("valid stroke");
    let via_stroke = render(&mut device, &stroked.finish(), 200, 200);

    let mut rect = SceneBuilder::new();
    rect.rect(
        Rect::new(Point::new(20.0, 97.0), Point::new(180.0, 103.0)),
        Affine::IDENTITY,
        Color::new(0.2, 0.2, 0.9, 1.0),
        None,
        None,
    )
    .expect("valid rect");
    let via_rect = render(&mut device, &rect.finish(), 200, 200);

    assert!(max_diff_premultiplied(&via_stroke, &via_rect) <= 1);
}

/// §11.4's bound holds for the new lanes too: a scene exercising glyphs, a curved
/// path, a stroke and a triangular residue clip differs across adapters by at most
/// the store-conversion bound — the coverage bytes themselves are CPU-made and
/// identical everywhere (ADR 0008).
#[test]
fn cross_adapter_bound_holds_for_coverage_lanes() {
    let adapters: Vec<String> = ["llvmpipe", "RADV"]
        .iter()
        .filter_map(|name| {
            Device::headless(&Options {
                adapter: Some((*name).into()),
                ..Options::default()
            })
            .ok()
            .map(|d| {
                drop(d);
                (*name).to_string()
            })
        })
        .collect();
    if adapters.len() < 2 {
        eprintln!("note: only one adapter; the cross-adapter check compared nothing");
        return;
    }

    let rasters: Vec<Vec<u8>> = adapters
        .iter()
        .map(|name| {
            let mut device = device_with(Options {
                adapter: Some(name.clone()),
                ..Options::default()
            });
            let glyph = device
                .upload_outline(&rect_outline(Rect::new(
                    Point::new(0.25, 0.25),
                    Point::new(5.5, 7.75),
                )))
                .expect("upload");
            let curve = device
                .upload_outline(&[
                    Segment::MoveTo(Point::new(10.0, 40.0)),
                    Segment::CubicTo {
                        c1: Point::new(30.0, 10.0),
                        c2: Point::new(50.0, 70.0),
                        to: Point::new(70.0, 40.0),
                    },
                    Segment::Close,
                ])
                .expect("upload");
            let triangle = device
                .upload_outline(&[
                    Segment::MoveTo(Point::new(0.0, 0.0)),
                    Segment::LineTo(Point::new(80.0, 0.0)),
                    Segment::LineTo(Point::new(40.0, 80.0)),
                    Segment::Close,
                ])
                .expect("upload");

            let mut builder = SceneBuilder::new();
            let clip = builder
                .clip(triangle, Affine::IDENTITY, FillRule::NonZero, None)
                .expect("valid clip");
            for i in 0..12_u32 {
                builder
                    .fill(
                        glyph,
                        Affine::translate(i as f32 * 6.37, 4.0),
                        FillRule::NonZero,
                        Paint::Solid(Color::new(0.1, 0.1, 0.4, 0.85)),
                        Some(clip),
                        BlendMode::Normal,
                        Compose::SrcOver,
                        None,
                    )
                    .expect("valid fill");
            }
            builder
                .fill(
                    curve,
                    Affine::IDENTITY,
                    FillRule::NonZero,
                    Paint::Solid(Color::new(0.7, 0.2, 0.2, 0.6)),
                    None,
                    BlendMode::Normal,
                    Compose::SrcOver,
                    None,
                )
                .expect("valid fill");
            builder
                .stroke(
                    curve,
                    Affine::IDENTITY,
                    Stroke {
                        width: 2.5,
                        cap: LineCap::Round,
                        join: LineJoin::Round,
                        miter_limit: 4.0,
                    },
                    Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0)),
                    None,
                    BlendMode::Normal,
                    None,
                )
                .expect("valid stroke");
            render(&mut device, &builder.finish(), 80, 80)
        })
        .collect();

    let diff = max_diff(&rasters[0], &rasters[1]);
    assert!(
        diff <= 2,
        "coverage lanes diverged across adapters by {diff} unorm steps (ADR 0006 bound: 2)"
    );
}

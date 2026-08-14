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
//!
//! ADR 0041 adds the second thing a frame can drop: a **child layer** whose clip leaves
//! it no pixel of its parent to contribute to. That is a claim about ISO 32000-2 clause
//! 11 rather than about the encoder, so the tests are written as the clause states each
//! composite — a group composited under §11.3.6, either half of §11.4.6's staged pair,
//! and §11.4.4's non-isolated group — and each says the same thing: **a group that can
//! reach no pixel leaves the page it was drawn on exactly as it found it.** The erase
//! half is the one worth reading twice, because getting it wrong subtracts rather than
//! adds, and a hole is what a reader would see.

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
    Affine, BlendMode, ClipId, Color, Compose, FillRule, GroupSpec, LineCap, LineJoin, Paint,
    Point, RampId, Rect, Scene, SceneBuilder, SceneError, Segment, ShadingKind, Stroke,
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
/// is derivable rather than observed: the shape covers half of column 0, and a black
/// fill on transparency reads back with that half as its alpha, `round(0.5 × 255)`.
///
/// **Both fill lanes are asked**, because they arrive at the byte by different
/// arithmetic and the edge is where a cull would show. The recognised rectangle takes
/// the analytic lane (ADR 0047), where the half is computed in `rect.wgsl` and rounded
/// once by the render target's unorm store; the same rectangle with a redundant vertex
/// on its top edge is not recognised, so it rasterises a coverage byte on the CPU
/// (ADR 0005/0008) that is then rounded again. Half a pixel survives both.
#[test]
fn a_fill_straddling_the_edge_keeps_its_coverage() {
    let mut device = device();
    let straddling = Rect::new(Point::new(-5.0, 6.0), Point::new(0.5, 26.0));
    let mut with_redundant_vertex = rect_outline(straddling);
    with_redundant_vertex.insert(
        1,
        Segment::LineTo(Point::new(
            (straddling.min.x + straddling.max.x) * 0.5,
            straddling.min.y,
        )),
    );
    for path in [rect_outline(straddling), with_redundant_vertex] {
        let outline = device.upload_outline(&path).unwrap();
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

// ------------------------------------------------- a child layer its clip leaves empty

/// The page every group below is drawn over: opaque, so that §11.4.6's erase would show
/// as a hole if it ran, and so that "nothing changed" is a statement about real pixels
/// rather than about transparency.
const PAGE: Rect = Rect {
    min: Point { x: 2.0, y: 2.0 },
    max: Point { x: 30.0, y: 30.0 },
};

/// What each group draws. Opaque and a different colour from the page, so that a group
/// that reaches the page cannot fail to change it.
const CONTENT: Rect = Rect {
    min: Point { x: 6.0, y: 6.0 },
    max: Point { x: 16.0, y: 16.0 },
};

/// A clip rectangle sharing no pixel with [`CONTENT`] — inside the target, so nothing
/// here is about the target's edge, and four pixels clear of the content, so nothing is
/// about the rounding at a shared boundary either.
const ELSEWHERE: Rect = Rect {
    min: Point { x: 20.0, y: 20.0 },
    max: Point { x: 28.0, y: 28.0 },
};

/// A clip rectangle that admits the whole of [`CONTENT`] — the control every test below
/// needs, since a cull that fired on both would prove nothing.
const AROUND_CONTENT: Rect = Rect {
    min: Point { x: 4.0, y: 4.0 },
    max: Point { x: 18.0, y: 18.0 },
};

fn page(builder: &mut SceneBuilder) {
    builder
        .rect(
            PAGE,
            Affine::IDENTITY,
            Color::new(0.9, 0.2, 0.1, 1.0),
            None,
            None,
        )
        .unwrap();
}

fn content(builder: &mut SceneBuilder) -> Result<(), SceneError> {
    builder.rect(
        CONTENT,
        Affine::IDENTITY,
        Color::new(0.1, 0.4, 0.9, 1.0),
        None,
        None,
    )
}

/// A rectangular clip, which resolves to one device rectangle and no residue — so the
/// only thing deciding whether the group can reach anything is `rect ∩ content`.
fn clip(device: &mut Device, builder: &mut SceneBuilder, rect: Rect) -> ClipId {
    let outline = device.upload_outline(&rect_outline(rect)).unwrap();
    builder
        .clip(outline, Affine::IDENTITY, FillRule::NonZero, None)
        .unwrap()
}

fn group(clip: ClipId) -> GroupSpec {
    GroupSpec {
        alpha: 1.0,
        blend: BlendMode::Normal,
        clip: Some(clip),
        knockout: false,
        mask: None,
        compose: Compose::SrcOver,
        isolated: true,
    }
}

/// The page on its own, which is what every "nothing changed" assertion compares to.
fn bare_page() -> Scene {
    let mut builder = SceneBuilder::new();
    page(&mut builder);
    builder.finish()
}

/// A group whose clip admits no pixel of what it draws contributes nothing, and is not
/// composited at all.
///
/// §11.3.6 composites the finished group with its backdrop weighted by the group's alpha,
/// soft mask and clip together; where the clip admits nothing that weight is zero, so
/// `co = ab·Cb` and `ao = ab` — the backdrop, unchanged. The frame must therefore be the
/// one the page alone produces, and the counters must say the group's whole rendering was
/// skipped rather than performed and discarded.
#[test]
fn a_group_its_clip_empties_is_never_composited() {
    let mut device = device();
    let (want, plain) = render(&mut device, &bare_page());
    assert_eq!(
        plain.layers_culled, 0,
        "a page with no group culls no layer"
    );

    let mut builder = SceneBuilder::new();
    page(&mut builder);
    let away = clip(&mut device, &mut builder, ELSEWHERE);
    builder.group(group(away), content).unwrap();
    let (got, counters) = render(&mut device, &builder.finish());

    assert_eq!(got, want, "a group that can reach no pixel changes none");
    assert_eq!(
        counters.layers_culled, 1,
        "and its layer was never rendered"
    );
    assert_eq!(
        counters.layer_textures, 1,
        "one texture, the root's accumulator: none was acquired for the group. It is 1 \
         rather than 0 because the culled plan stays in the frame's layer list, which \
         keeps the frame off the flat fast path (ADR 0041's stated cost)"
    );

    // The control: the same group under a clip that admits it draws, and costs what the
    // cull saved — the root's accumulator, the group's, and the copy of the backdrop the
    // composite reads (ADR 0038).
    let mut builder = SceneBuilder::new();
    page(&mut builder);
    let around = clip(&mut device, &mut builder, AROUND_CONTENT);
    builder.group(group(around), content).unwrap();
    let (drawn, control) = render(&mut device, &builder.finish());

    assert_eq!(control.layers_culled, 0, "this group reaches the page");
    assert_eq!(control.layer_textures, 3);
    assert_ne!(drawn, want, "and so it changes it");
}

/// **An erase weighted by a shape that is zero everywhere erases nothing**, and a deposit
/// of nothing deposits nothing — ISO 32000-2 §11.4.6's two stages, each asked for by name
/// on a group (ADR 0033).
///
/// The clause's stage is `P' = (1 − f) × P + S`. A group standing for the erase half
/// contributes `f`, and one standing for the deposit half contributes `S`; a group its
/// clip empties contributes zero to either, leaving `P' = P`. This is the case where a
/// wrong cull would *subtract* — the erase is the only composite in the clause that can
/// remove what is already on the page — so the control asserts that the same group, under
/// a clip that admits it, really does punch the hole.
#[test]
fn a_staged_group_its_clip_empties_neither_erases_nor_deposits() {
    let mut device = device();
    let (want, _) = render(&mut device, &bare_page());

    for stage in [Compose::DestOut, Compose::Plus] {
        let mut builder = SceneBuilder::new();
        page(&mut builder);
        let away = clip(&mut device, &mut builder, ELSEWHERE);
        builder
            .group(
                GroupSpec {
                    compose: stage,
                    ..group(away)
                },
                content,
            )
            .unwrap();
        let (got, counters) = render(&mut device, &builder.finish());

        assert_eq!(
            got, want,
            "{stage:?} of a group that reaches nothing is P' = P"
        );
        assert_eq!(counters.layers_culled, 1);
    }

    // The control, for the half that removes: this erase reaches the page, and where its
    // opaque content lies the page is gone — `P' = (1 − 1) × P`.
    let mut builder = SceneBuilder::new();
    page(&mut builder);
    let around = clip(&mut device, &mut builder, AROUND_CONTENT);
    builder
        .group(
            GroupSpec {
                compose: Compose::DestOut,
                ..group(around)
            },
            content,
        )
        .unwrap();
    let (erased, control) = render(&mut device, &builder.finish());

    assert_eq!(control.layers_culled, 0);
    assert_eq!(
        alpha_at(&erased, 10, 10),
        0,
        "an erase by an opaque shape leaves nothing behind it"
    );
    assert_eq!(
        alpha_at(&want, 10, 10),
        255,
        "which is a change, because the page is opaque there"
    );
}

/// §11.4.4's non-isolated group is the case the compositor cannot catch for itself, and
/// culling it is still exact.
///
/// A non-isolated group's buffer is seeded with a texel-for-texel copy of its backdrop,
/// so it takes its parent's whole region (ADR 0038) and always meets it — the
/// compositor's own "this child reaches nothing" test can never fire for one. It is exact
/// all the same: wherever the group marked nothing its buffer *is* the backdrop, and the
/// clause's `(1 − w) × B + w × E(B)` with `E(B) = B` is `B` for every weight.
#[test]
fn a_non_isolated_group_its_clip_empties_leaves_its_backdrop() {
    let mut device = device();
    let (want, _) = render(&mut device, &bare_page());

    let mut builder = SceneBuilder::new();
    page(&mut builder);
    let away = clip(&mut device, &mut builder, ELSEWHERE);
    builder
        .group(
            GroupSpec {
                isolated: false,
                ..group(away)
            },
            content,
        )
        .unwrap();
    let (got, counters) = render(&mut device, &builder.finish());

    assert_eq!(got, want, "the backdrop, as §11.4.4 leaves it");
    assert_eq!(counters.layers_culled, 1);

    let mut builder = SceneBuilder::new();
    page(&mut builder);
    let around = clip(&mut device, &mut builder, AROUND_CONTENT);
    builder
        .group(
            GroupSpec {
                isolated: false,
                ..group(around)
            },
            content,
        )
        .unwrap();
    let (drawn, control) = render(&mut device, &builder.finish());

    assert_eq!(control.layers_culled, 0);
    assert_ne!(drawn, want);
}

/// **A group that marks nothing is a different case from one a clip emptied**, and both
/// are dropped.
///
/// Nothing here is clipped away — both groups are clipped to the whole page. The first
/// has no elements; the second's one element is a rectangle with no area, a well-formed
/// command that covers no pixel, which is what a real page produces from a collapsed
/// transform. Their layers hold no bounds at all rather than bounds their clip misses, so
/// this is the arm that reaches `bounds == None`, and the frame it draws is the page on
/// its own.
#[test]
fn a_group_that_marks_nothing_is_dropped_too() {
    let mut device = device();
    let (want, _) = render(&mut device, &bare_page());

    let mut builder = SceneBuilder::new();
    page(&mut builder);
    let over_the_page = clip(&mut device, &mut builder, PAGE);
    builder.group(group(over_the_page), |_| Ok(())).unwrap();
    builder
        .group(group(over_the_page), |inner| {
            inner.rect(
                Rect::new(Point::new(10.0, 10.0), Point::new(10.0, 20.0)),
                Affine::IDENTITY,
                Color::new(0.1, 0.4, 0.9, 1.0),
                None,
                None,
            )
        })
        .unwrap();
    let (got, counters) = render(&mut device, &builder.finish());

    assert_eq!(got, want, "neither group marks anything");
    assert_eq!(
        counters.layers_culled, 2,
        "and neither is composited: an empty group, and one whose element has no area"
    );
    assert_eq!(
        counters.commands_culled, 1,
        "the two counters count different things and this scene has one of each: the \
         rectangle with no area is a command that reaches no pixel, and the groups \
         above it are layers with nothing to composite"
    );
}

//! Which lane a command takes, asked through the walk's own entry point.
//!
//! Every test here builds a scene, calls [`encode`](super::encode) and reads the lane
//! back out of what it returned — which instance stream grew, what kind of batch the op
//! is, whether a scratch sheet exists at all. That is the subject rather than an
//! artefact of testing from outside: a lane is a *choice* between several ways of
//! drawing one mark, and the encoding is the only place the choice is visible. The two
//! refusal tests belong to the same subject for the same reason — a budget checked
//! before allocation, and a blank scene that is still a scene, are properties of the
//! entry point rather than of any one arm.
//!
//! Nothing here touches wgpu. What a lane's pixels look like is gated on a device, in
//! `tests/`.

use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, Paint, Point, Rect, SceneBuilder, Segment,
};

use super::{BatchKind, encode};
use crate::atlas::AtlasStore;
use crate::error::RenderError;
use crate::resources::ResourceStore;
use crate::startup::Coverage;
use crate::viewport::Viewport;

fn no_resources() -> ResourceStore {
    ResourceStore::new(0)
}

fn empty_atlas() -> AtlasStore {
    AtlasStore::new(1024, 1024)
}

fn scene_with_one_rect() -> quorra_scene::Scene {
    let mut builder = SceneBuilder::new();
    builder
        .rect(
            Rect::new(Point::new(1.0, 2.0), Point::new(3.0, 5.0)),
            Affine::IDENTITY,
            Color::new(1.0, 0.5, 0.0, 0.5),
            None,
            None,
        )
        .expect("valid input");
    builder.finish()
}

/// One rect, identity viewport: one instance, one rect batch, premultiplied
/// colour at bytes 16..32.
#[test]
fn encodes_one_instance_with_premultiplied_color() {
    let scene = scene_with_one_rect();
    let viewport = Viewport::full(10, 10, Affine::IDENTITY);
    let encoded = encode(
        &scene,
        &viewport,
        u64::MAX,
        4096,
        &no_resources(),
        &mut empty_atlas(),
        Some(16),
        Coverage::Cpu,
        crate::startup::DEFAULT_COVERAGE_SAMPLES,
        false,
        1,
    )
    .expect("within budget");
    assert_eq!(encoded.commands, 1);
    assert_eq!(encoded.rect_instances.len(), 32);
    assert_eq!(encoded.root.ops.len(), 1);
    assert!(matches!(
        encoded.root.ops[0],
        super::Op::Draw(super::Batch {
            kind: BatchKind::Rect,
            ..
        })
    ));
    let read_f32 = |offset: usize| {
        let bytes: [u8; 4] = encoded.rect_instances[offset..offset + 4]
            .try_into()
            .expect("in bounds");
        f32::from_le_bytes(bytes)
    };
    assert!((read_f32(16) - 0.5).abs() < 1e-6);
    assert!((read_f32(20) - 0.25).abs() < 1e-6);
    assert!((read_f32(24) - 0.0).abs() < 1e-6);
    assert!((read_f32(28) - 0.5).abs() < 1e-6);
}

/// An oblique rectangle no longer refuses: it takes the path lane and comes back
/// as a scratch quad.
#[test]
fn oblique_rect_takes_the_path_lane() {
    let mut builder = SceneBuilder::new();
    let shear = Affine {
        a: 1.0,
        b: 0.3,
        c: 0.0,
        d: 1.0,
        e: 2.0,
        f: 2.0,
    };
    builder
        .rect(
            Rect::new(Point::new(0.0, 0.0), Point::new(4.0, 4.0)),
            shear,
            Color::new(0.0, 0.0, 0.0, 1.0),
            None,
            None,
        )
        .expect("valid rect");
    let scene = builder.finish();
    let viewport = Viewport::full(16, 16, Affine::IDENTITY);
    let encoded = encode(
        &scene,
        &viewport,
        u64::MAX,
        4096,
        &no_resources(),
        &mut empty_atlas(),
        Some(16),
        Coverage::Cpu,
        crate::startup::DEFAULT_COVERAGE_SAMPLES,
        false,
        1,
    )
    .expect("drawable since M5");
    assert_eq!(encoded.root.ops.len(), 1);
    assert!(matches!(
        encoded.root.ops[0],
        super::Op::Draw(super::Batch {
            kind: BatchKind::Quad,
            ..
        })
    ));
    assert!(encoded.scratch.is_some());
}

/// A store holding one outline: the four axis-aligned edges of `1,2 → 3,5`, which
/// is what `quorra_scene::axis_aligned_rect` recognises and what the analytic lane
/// exists for.
fn store_with_a_rectangle() -> (ResourceStore, quorra_scene::OutlineId) {
    let mut resources = ResourceStore::new(4_096);
    let outline = resources
        .upload_outline(&[
            Segment::MoveTo(Point::new(1.0, 2.0)),
            Segment::LineTo(Point::new(3.0, 2.0)),
            Segment::LineTo(Point::new(3.0, 5.0)),
            Segment::LineTo(Point::new(1.0, 5.0)),
            Segment::Close,
        ])
        .expect("a rectangle within the store's budget");
    (resources, outline)
}

/// One solid fill of `outline` under `transform`, and nothing else.
fn scene_filling(
    outline: quorra_scene::OutlineId,
    transform: Affine,
    rule: FillRule,
) -> quorra_scene::Scene {
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            transform,
            rule,
            Paint::Solid(Color::new(1.0, 0.5, 0.0, 0.5)),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("valid input");
    builder.finish()
}

/// A solid fill whose outline is four axis-aligned edges takes the rectangle lane
/// (ADR 0047) — the same instance bytes `Command::Rect` produces for the same mark,
/// with no atlas key and no coverage tile behind it.
#[test]
fn a_solid_fill_of_a_rectangle_takes_the_rectangle_lane() {
    let (resources, outline) = store_with_a_rectangle();
    let scene = scene_filling(outline, Affine::IDENTITY, FillRule::NonZero);
    let viewport = Viewport::full(10, 10, Affine::IDENTITY);
    let encoded = encode(
        &scene,
        &viewport,
        u64::MAX,
        4096,
        &resources,
        &mut empty_atlas(),
        Some(16),
        Coverage::Cpu,
        crate::startup::DEFAULT_COVERAGE_SAMPLES,
        false,
        1,
    )
    .expect("within budget");
    assert!(matches!(
        encoded.root.ops.as_slice(),
        [super::Op::Draw(super::Batch {
            kind: BatchKind::Rect,
            count: 1,
            ..
        })]
    ));
    assert!(encoded.quad_instances.is_empty());
    assert!(encoded.scratch.is_none());
    assert_eq!(encoded.atlas_distinct_keys, 0);
    assert_eq!(encoded.tiles, 0);

    // The bytes are `Command::Rect`'s for the same rectangle and colour: this is
    // one lane reached through two doors, not two lanes that agree.
    let same_mark = {
        let mut builder = SceneBuilder::new();
        builder
            .rect(
                Rect::new(Point::new(1.0, 2.0), Point::new(3.0, 5.0)),
                Affine::IDENTITY,
                Color::new(1.0, 0.5, 0.0, 0.5),
                None,
                None,
            )
            .expect("valid input");
        builder.finish()
    };
    let reference = encode(
        &same_mark,
        &viewport,
        u64::MAX,
        4096,
        &no_resources(),
        &mut empty_atlas(),
        Some(16),
        Coverage::Cpu,
        crate::startup::DEFAULT_COVERAGE_SAMPLES,
        false,
        1,
    )
    .expect("within budget");
    assert_eq!(encoded.rect_instances, reference.rect_instances);

    // And the fill rule cannot change it: one closed subpath of four corners bounds
    // the same region under §8.5.3.3.2 and §8.5.3.3.3, which is why the lane does
    // not ask.
    let even_odd = encode(
        &scene_filling(outline, Affine::IDENTITY, FillRule::EvenOdd),
        &viewport,
        u64::MAX,
        4096,
        &resources,
        &mut empty_atlas(),
        Some(16),
        Coverage::Cpu,
        crate::startup::DEFAULT_COVERAGE_SAMPLES,
        false,
        1,
    )
    .expect("within budget");
    assert_eq!(even_odd.rect_instances, encoded.rect_instances);
}

/// The lane's conditions bite: an oblique transform makes the same outline a
/// parallelogram, which `rect.wgsl` cannot express, so it keeps the path lane.
#[test]
fn an_oblique_fill_of_a_rectangle_keeps_the_path_lane() {
    let (resources, outline) = store_with_a_rectangle();
    let shear = Affine {
        a: 1.0,
        b: 0.3,
        c: 0.0,
        d: 1.0,
        e: 2.0,
        f: 2.0,
    };
    let encoded = encode(
        &scene_filling(outline, shear, FillRule::NonZero),
        &Viewport::full(16, 16, Affine::IDENTITY),
        u64::MAX,
        4096,
        &resources,
        &mut empty_atlas(),
        Some(16),
        Coverage::Cpu,
        crate::startup::DEFAULT_COVERAGE_SAMPLES,
        false,
        1,
    )
    .expect("drawable");
    assert!(encoded.rect_instances.is_empty());
    assert!(!encoded.quad_instances.is_empty());
}

/// The budget is checked before allocation, and the error names both numbers.
#[test]
fn budget_is_checked_before_allocation() {
    let scene = scene_with_one_rect();
    let viewport = Viewport::full(10, 10, Affine::IDENTITY);
    match encode(
        &scene,
        &viewport,
        16,
        4096,
        &no_resources(),
        &mut empty_atlas(),
        Some(16),
        Coverage::Cpu,
        crate::startup::DEFAULT_COVERAGE_SAMPLES,
        false,
        1,
    ) {
        Err(RenderError::FrameBudgetExceeded { needed, budget }) => {
            assert_eq!(needed, 96);
            assert_eq!(budget, 16);
        }
        other => panic!("expected FrameBudgetExceeded, got {other:?}"),
    }
}

/// A blank scene encodes to zero instances and zero batches, without error.
#[test]
fn blank_scene_encodes_to_nothing() {
    let scene = SceneBuilder::new().finish();
    let viewport = Viewport::full(10, 10, Affine::IDENTITY);
    let encoded = encode(
        &scene,
        &viewport,
        u64::MAX,
        4096,
        &no_resources(),
        &mut empty_atlas(),
        Some(16),
        Coverage::Cpu,
        crate::startup::DEFAULT_COVERAGE_SAMPLES,
        false,
        1,
    )
    .expect("blank is legitimate");
    assert_eq!(encoded.commands, 0);
    assert!(encoded.root.ops.is_empty());
    assert!(encoded.scratch.is_none());
}

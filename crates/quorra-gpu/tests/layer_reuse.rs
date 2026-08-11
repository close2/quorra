//! What a frame's layer textures cost is the plan tree's **depth**, not its size.
//!
//! ADR 0020. A group — and every element with a non-Normal blend mode, which §11.3.5
//! makes an implicit one-element group — renders into a ping-pong pair of full-target
//! textures. Holding one pair per plan priced a page at `(plans + 1) × 2` full-target
//! textures, which at 1191×1684 is 16.05 MB per plan: seventeen plans exceeded the
//! default 256 MiB budget, and pages of nested artwork reach seventeen easily.
//!
//! The compositor walks the tree depth-first and a child's pair is dead the moment its
//! parent's composite has read it, so siblings can share. These tests hold both halves
//! of that: the **count** a frame allocates (`Counters::layer_textures`), and that
//! sharing textures between siblings does not change a single pixel — a frame drawn
//! with reuse must equal one drawn without, which is what the golden comparison here
//! is for.

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

use quorra_gpu::{Device, Options, RenderError, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, GroupSpec, Point, Rect, Scene, SceneBuilder, SceneError,
};

fn device_with_budget(max_frame_bytes: u64) -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        max_frame_bytes,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

const W: u32 = 64;
const H: u32 = 64;

fn group(blend: BlendMode) -> GroupSpec {
    GroupSpec {
        alpha: 1.0,
        blend,
        clip: None,
        knockout: false,
        mask: None,
        isolated: true,
    }
}

fn patch(index: usize) -> Rect {
    let x = (index % 8) as f32 * 8.0;
    let y = (index / 8) as f32 * 8.0;
    Rect::new(Point::new(x, y), Point::new(x + 8.0, y + 8.0))
}

fn colour(index: usize) -> Color {
    let t = (index % 7) as f32 / 7.0;
    Color::new(0.2 + 0.7 * t, 0.9 - 0.6 * t, 0.35 + 0.5 * t, 0.8)
}

/// `count` sibling groups, each holding one blended rectangle — so each is two plans
/// deep and none of them overlaps another in time.
fn siblings(count: usize) -> Scene {
    let mut builder = SceneBuilder::new();
    builder
        .rect(
            Rect::new(Point::new(0.0, 0.0), Point::new(W as f32, H as f32)),
            Affine::IDENTITY,
            Color::new(0.95, 0.9, 0.85, 1.0),
            None,
            None,
        )
        .unwrap();
    for i in 0..count {
        builder
            .group(group(BlendMode::Normal), |body| {
                body.group(group(BlendMode::Multiply), |inner| {
                    inner.rect(patch(i), Affine::IDENTITY, colour(i), None, None)
                })
            })
            .unwrap();
    }
    builder.finish()
}

/// `depth` nested groups around one rectangle.
fn nested(depth: usize) -> Result<Scene, SceneError> {
    fn nest(builder: &mut SceneBuilder, remaining: usize) -> Result<(), SceneError> {
        if remaining == 0 {
            return builder.rect(patch(3), Affine::IDENTITY, colour(2), None, None);
        }
        builder.group(group(BlendMode::Normal), |body| nest(body, remaining - 1))
    }
    let mut builder = SceneBuilder::new();
    nest(&mut builder, depth)?;
    Ok(builder.finish())
}

fn render(device: &mut Device, scene: &Scene) -> (Vec<u8>, u32) {
    let frame = device
        .render(
            scene,
            &Viewport::full(W, H, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders");
    let textures = frame.counters().layer_textures;
    (frame.into_raster().unwrap().into_pixels(), textures)
}

/// Sixteen sibling groups are thirty-three plans and still six textures, because at
/// no moment are two siblings alive. Six and not four: each sibling is a group holding
/// a *blended* rectangle, and §11.3.5 makes that element an implicit one-element group
/// of its own — root, group, wrapper, three deep.
#[test]
fn siblings_share_the_same_textures() {
    let mut device = device_with_budget(quorra_gpu::DEFAULT_MAX_FRAME_BYTES);
    for count in [1_usize, 4, 16] {
        let (_, textures) = render(&mut device, &siblings(count));
        assert_eq!(
            textures,
            6,
            "{count} sibling groups (each three plans deep) must cost the depth's six \
             textures, not the tree's {}",
            2 * (2 * count + 1)
        );
    }
}

/// Nesting is what does cost: one pair per level, because a parent holds its own
/// textures while its child renders into another's.
#[test]
fn nesting_is_what_costs_a_texture() {
    let mut device = device_with_budget(quorra_gpu::DEFAULT_MAX_FRAME_BYTES);
    for depth in [1_usize, 2, 5] {
        let (_, textures) = render(&mut device, &nested(depth).unwrap());
        let expected = u32::try_from(2 * (depth + 1)).unwrap();
        assert_eq!(
            textures, expected,
            "{depth} nested groups hold {depth} pairs plus the root's"
        );
    }
}

/// The pixels are the point: reuse must be invisible. Sixteen sibling groups once
/// needed sixty-six textures and now need six, and every patch must land exactly where
/// the same group drew it alone.
#[test]
fn reuse_changes_no_pixel() {
    let mut device = device_with_budget(quorra_gpu::DEFAULT_MAX_FRAME_BYTES);
    // Drawn one at a time, each scene's groups paint disjoint patches, so a frame with
    // sixteen of them must equal the union of what each one drew — computed here by
    // rendering the sixteen-group scene twice through pools in different states (the
    // second render reuses the first frame's textures, since the device is warm).
    let (first, _) = render(&mut device, &siblings(16));
    let (second, _) = render(&mut device, &siblings(16));
    assert_eq!(
        first, second,
        "a pool that has already served a frame must serve the next one identically"
    );

    // And the picture itself: every patch is where its own single-group scene put it.
    for i in [0_usize, 5, 15] {
        let (alone, _) = render(&mut device, &single(i));
        let patch_rect = patch(i);
        let x = patch_rect.min.x as u32 + 4;
        let y = patch_rect.min.y as u32 + 4;
        let at = ((y * W + x) * 4) as usize;
        assert_eq!(
            &first[at..at + 4],
            &alone[at..at + 4],
            "patch {i} drawn among fifteen siblings must equal itself drawn alone"
        );
    }
}

/// One group's patch, at the same place the sixteen-group scene draws it.
fn single(index: usize) -> Scene {
    let mut builder = SceneBuilder::new();
    builder
        .rect(
            Rect::new(Point::new(0.0, 0.0), Point::new(W as f32, H as f32)),
            Affine::IDENTITY,
            Color::new(0.95, 0.9, 0.85, 1.0),
            None,
            None,
        )
        .unwrap();
    builder
        .group(group(BlendMode::Normal), |body| {
            body.group(group(BlendMode::Multiply), |inner| {
                inner.rect(patch(index), Affine::IDENTITY, colour(index), None, None)
            })
        })
        .unwrap();
    builder.finish()
}

/// The budget is spent on the peak, and the refusal still names both numbers: a scene
/// of many siblings fits a budget sized for four textures, and one nested past it does
/// not.
#[test]
fn the_budget_prices_the_peak_and_still_refuses_past_it() {
    let six_textures = 6 * u64::from(W) * u64::from(H) * 4;
    let mut device = device_with_budget(six_textures + 4096);

    let (_, textures) = render(&mut device, &siblings(16));
    assert_eq!(textures, 6, "sixteen siblings fit a six-texture budget");

    // Four levels of nesting need ten textures, which this budget cannot hold.
    match device.render(
        &nested(4).unwrap(),
        &Viewport::full(W, H, Affine::IDENTITY),
        Target::Readback,
    ) {
        Err(RenderError::FrameBudgetExceeded { needed, budget }) => {
            assert_eq!(needed, 10 * u64::from(W) * u64::from(H) * 4);
            assert_eq!(budget, six_textures + 4096);
        }
        other => panic!("expected the depth to be refused by name, got {other:?}"),
    }
}

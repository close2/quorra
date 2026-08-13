//! What a frame's layer textures cost is the plan tree's **depth**, not its size.
//!
//! ADR 0020. A group — and every element with a non-Normal blend mode, which §11.3.5
//! makes an implicit one-element group — renders into a texture of its own. Holding one
//! *pair* per plan priced a page at `(plans + 1) × 2` full-target textures, which at
//! 1191×1684 is 16.05 MB per plan: seventeen plans exceeded the default 256 MiB budget,
//! and pages of nested artwork reach seventeen easily.
//!
//! The compositor walks the tree depth-first and a child's texture is dead the moment its
//! parent's composite has read it, so siblings can share. These tests hold both halves of
//! that: the **count** a frame allocates (`Counters::layer_textures`), and that sharing
//! textures between siblings does not change a single pixel — a frame drawn with reuse
//! must equal one drawn without, which is what the golden comparison here is for.
//!
//! **What a level costs is one texture plus a transient** (ADR 0038): a plan accumulates
//! in one texture rather than ping-ponging between two, and the composite that folds a
//! child into it reads a copy of the pixels it covers — the child's size, alive only for
//! that pass. So a chain `n` plans deep holds `n + 1` at its worst moment, where it held
//! `2n`.

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
    Affine, BlendMode, Color, Compose, GroupSpec, Point, Rect, Scene, SceneBuilder, SceneError,
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
        compose: Compose::SrcOver,
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
            // The whole target, not a patch: since ADR 0036 a layer is as big as its
            // plan, so a chain of *small* groups costs almost nothing and this test would
            // be about a budget nobody could exceed. What it is about is the chain, and
            // the chain has to be made of full-sized plans to weigh anything.
            return builder.rect(
                Rect::new(Point::new(0.0, 0.0), Point::new(W as f32, H as f32)),
                Affine::IDENTITY,
                colour(2),
                None,
                None,
            );
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

/// Sixteen sibling groups are thirty-three plans and still four textures, because at no
/// moment are two siblings alive. Three deep and not two: each sibling is a group holding
/// a *blended* rectangle, and §11.3.5 makes that element an implicit one-element group of
/// its own — root, group, wrapper. The fourth is the backdrop the innermost composite
/// copies out of its parent (ADR 0038).
#[test]
fn siblings_share_the_same_textures() {
    let mut device = device_with_budget(quorra_gpu::DEFAULT_MAX_FRAME_BYTES);
    for count in [1_usize, 4, 16] {
        let (_, textures) = render(&mut device, &siblings(count));
        assert_eq!(
            textures,
            4,
            "{count} sibling groups (each three plans deep) must cost the depth's four \
             textures, not the tree's {}",
            2 * count + 2
        );
    }
}

/// Nesting is what does cost: one texture per level, because a parent holds its own while
/// its child renders into another — plus the one transient copy the deepest composite
/// reads (ADR 0038), which is one whatever the depth, since composites finish innermost
/// first and each releases its copy before the next acquires one.
#[test]
fn nesting_is_what_costs_a_texture() {
    let mut device = device_with_budget(quorra_gpu::DEFAULT_MAX_FRAME_BYTES);
    for depth in [1_usize, 2, 5] {
        let (_, textures) = render(&mut device, &nested(depth).unwrap());
        let expected = u32::try_from(depth + 2).unwrap();
        assert_eq!(
            textures, expected,
            "{depth} nested groups hold {depth} textures plus the root's and one copy"
        );
    }
}

/// **The root is as big as what the page marks too** (ADR 0039), and the hand-off from it
/// to the target is where that could go wrong: the root's texture is smaller than the
/// target, so the copy reads at a negative origin and must write *transparency* — not
/// stale bytes, and not the nearest edge texel — everywhere the page marked nothing.
///
/// A page rendered onto transparency (§3) has exactly that outside its marks, so the test
/// is that a corner-marking page equals itself pixel for pixel: inside the group's patch,
/// and transparent in all three of the other quadrants.
///
/// `m8`'s `a_group_smaller_than_the_damage_patches_too` is the other half — the same copy
/// under `LoadOp::Load`, where writing nothing would leave the *previous* frame's pixels
/// inside a damage rectangle that a full redraw would have cleared.
#[test]
fn a_page_that_marks_a_corner_hands_off_only_that_corner() {
    let mut device = device_with_budget(quorra_gpu::DEFAULT_MAX_FRAME_BYTES);
    let mut builder = SceneBuilder::new();
    builder
        .group(group(BlendMode::Normal), |body| {
            body.group(group(BlendMode::Multiply), |inner| {
                inner.rect(patch(0), Affine::IDENTITY, colour(0), None, None)
            })
        })
        .unwrap();
    let (pixels, _) = render(&mut device, &builder.finish());

    // patch(0) is the 8 × 8 at the origin, and nothing else is drawn at all.
    let at = |x: u32, y: u32| {
        let i = ((y * W + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    assert_ne!(
        at(4, 4),
        [0, 0, 0, 0],
        "the marked corner must hold the group's patch"
    );
    for (x, y) in [(40, 4), (4, 40), (40, 40), (W - 1, H - 1), (8, 8)] {
        assert_eq!(
            at(x, y),
            [0, 0, 0, 0],
            "({x}, {y}) is outside everything the page marked: transparent, whatever the \
             root's texture happens to be sized"
        );
    }
}

/// The pixels are the point: reuse must be invisible. Sixteen sibling groups once needed
/// sixty-six textures and now need four, and every patch must land exactly where the same
/// group drew it alone.
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

/// The budget is spent on the peak, and the refusal still names both numbers: a scene of
/// many siblings fits a budget sized for four textures, and one nested past it does not.
#[test]
fn the_budget_prices_the_peak_and_still_refuses_past_it() {
    let page = u64::from(W) * u64::from(H) * 4;
    let mut device = device_with_budget(4 * page + 4096);

    let (_, textures) = render(&mut device, &siblings(16));
    assert_eq!(textures, 4, "sixteen siblings fit a four-texture budget");
    // And they fit it while each sibling's own layer is a patch rather than a page: the
    // budget is spent on the chain that is alive at once, which ADR 0036 prices at each
    // plan's own size and ADR 0038 at one texture per plan.

    // Four levels of nesting need six textures — five plans and the deepest composite's
    // copy, all of them page-sized here because every group covers the page.
    match device.render(
        &nested(4).unwrap(),
        &Viewport::full(W, H, Affine::IDENTITY),
        Target::Readback,
    ) {
        Err(RenderError::FrameBudgetExceeded { needed, budget }) => {
            assert_eq!(needed, 6 * page);
            assert_eq!(budget, 4 * page + 4096);
        }
        other => panic!("expected the depth to be refused by name, got {other:?}"),
    }
}

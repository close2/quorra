//! Every entry of ADR 0045's invalidation list, one test per entry.
//!
//! ADR 0048's `Device::render_retained` replays an encode when nothing an encode reads
//! has changed. What "nothing" means is a list, and a list is only as good as the
//! enumeration of it — so this file is written the way the list is: **one test per entry,
//! named for the entry**, each asserting the observable `Frame::encode_source`. Two of
//! them assert a *hit* rather than a miss (the damage list and the target), and those are
//! entries of the list too: a thing that deliberately does not invalidate has to be
//! stated, or the next reader will add it.
//!
//! An entry missing from here is an encode that survives a change it should not survive,
//! which is a plausible-looking wrong page. That the pixels of a surviving encode are the
//! right pixels is `retained_replay.rs`; the atlas-overflow shape is `retained_atlas.rs`.

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

use quorra_gpu::{
    Coverage, EncodeSource, Options, RenderError, RetainedScene, Target, Viewport, wgpu,
};
use quorra_scene::{Affine, BlendMode, Color, Compose, FillRule, Paint, Point, Rect, SceneBuilder};

mod common;

use common::retained::{
    H, W, artwork_page, device, device_with, place, retained_frame, text_page, viewport,
};

/// **The scene.** A handle given a new scene drops its encode, even for a scene built
/// identically: identity is what the handle can check, and a miss costs one encode where
/// a wrong hit costs a wrong page.
#[test]
fn a_new_scene_re_encodes() {
    let mut device = device();
    let (scene, outlines) = text_page(&mut device, 60, 8, 10.0);
    let viewport = viewport();
    let mut retained = RetainedScene::new(scene);
    retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "first frame",
    );
    retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Replayed,
        "unchanged",
    );

    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outlines[0],
            place(0, 11.0),
            FillRule::NonZero,
            Paint::Solid(Color::new(1.0, 0.0, 0.0, 1.0)),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    retained.set_scene(builder.finish());
    assert!(
        !retained.holds_encode(),
        "a new scene drops the encode at the moment it is handed over, not at the next frame"
    );
    retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "a different scene is a different frame",
    );
}

/// **The viewport's size.** Every cull and every clip rectangle is tested against the
/// target rectangle, so a resize is a different encode.
#[test]
fn a_resized_viewport_re_encodes() {
    let mut device = device();
    let (scene, _) = text_page(&mut device, 60, 8, 10.0);
    let mut retained = RetainedScene::new(scene);
    let first = Viewport::full(W, H, Affine::IDENTITY);
    let taller = Viewport::full(W, H + 1, Affine::IDENTITY);
    retained_frame(
        &mut device,
        &mut retained,
        &first,
        EncodeSource::Encoded,
        "first frame",
    );
    retained_frame(
        &mut device,
        &mut retained,
        &taller,
        EncodeSource::Encoded,
        "one more row of pixels is a different target rectangle",
    );
    retained_frame(
        &mut device,
        &mut retained,
        &first,
        EncodeSource::Encoded,
        "and going back is another one — one encode is retained, not a cache of them",
    );
}

/// **The viewport's affine.** A scroll of a fraction of a pixel moves the quantised
/// sub-pixel phase, so every atlas key changes (ADR 0009); a whole-pixel scroll leaves
/// the tiles valid but moves every absolute device position. Both re-encode, and ADR
/// 0045's survival table is why neither can do better.
#[test]
fn a_moved_viewport_re_encodes() {
    let mut device = device();
    let (scene, _) = text_page(&mut device, 60, 8, 10.0);
    let mut retained = RetainedScene::new(scene);
    let still = Viewport::full(W, H, Affine::IDENTITY);
    let scrolled = Viewport::full(W, H, Affine::translate(0.0, -3.0));
    let nudged = Viewport::full(W, H, Affine::translate(0.0, -3.25));
    let zoomed = Viewport::full(W, H, Affine::scale(1.5, 1.5));
    for (viewport, why) in [
        (&still, "first frame"),
        (&scrolled, "a whole-pixel scroll moves every device bound"),
        (&nudged, "a fractional scroll moves every sub-pixel phase"),
        (
            &zoomed,
            "a zoom step is a different rasterisation of every shape",
        ),
    ] {
        retained_frame(
            &mut device,
            &mut retained,
            viewport,
            EncodeSource::Encoded,
            why,
        );
    }
}

/// **The damage list does not invalidate.** `encode` never reads it — damage is planned
/// target-side (ADR 0012) — so a damaged frame replays a full frame's encode. This is
/// the one entry of the list that asserts a *hit*, and it is the one the caller's scroll
/// path depends on.
#[test]
fn a_changed_damage_list_replays() {
    let mut device = device();
    let (scene, _) = text_page(&mut device, 60, 8, 10.0);
    let (width, height) = (W, H);
    let mut retained = RetainedScene::new(scene);
    let full = Viewport::full(width, height, Affine::IDENTITY);
    retained_frame(
        &mut device,
        &mut retained,
        &full,
        EncodeSource::Encoded,
        "first frame",
    );

    // A `Readback` target has no retained contents to patch, so the damage list is
    // reported as not honoured and the whole target is redrawn — which is exactly the
    // point here: the *encode* did not care either way.
    let damage = [Rect::new(Point::new(0.0, 0.0), Point::new(32.0, 32.0))];
    let damaged = Viewport {
        width,
        height,
        transform: Affine::IDENTITY,
        damage: &damage,
    };
    let frame = retained_frame(
        &mut device,
        &mut retained,
        &damaged,
        EncodeSource::Replayed,
        "damage is planned target-side; the encode is unchanged",
    );
    assert_eq!(
        frame.reports().len(),
        1,
        "a `Readback` target says it could not honour the damage list"
    );
}

/// **The target does not invalidate.** Phase 1 runs before any allocation and knows no
/// target, so the same encode draws into a readback texture and into the caller's own.
#[test]
fn a_different_target_replays() {
    let mut device = device();
    let (scene, _) = text_page(&mut device, 60, 8, 10.0);
    let viewport = viewport();
    let mut retained = RetainedScene::new(scene);
    retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "first frame",
    );

    let (gpu, _) = device.wgpu();
    let texture = gpu.create_texture(&wgpu::TextureDescriptor {
        label: Some("retained target"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let frame = device
        .render_retained(&mut retained, &viewport, Target::Texture(&texture))
        .expect("the frame must draw");
    assert_eq!(
        frame.encode_source(),
        EncodeSource::Replayed,
        "a target is not an input to phase 1"
    );
}

/// **The coverage lane.** `Device::set_coverage` chooses which lane makes coverage
/// bytes, per frame by design (ADR 0016), and the two lanes' bytes differ within a
/// stated bound — so an encode made under one must never be replayed under the other.
#[test]
fn a_changed_coverage_lane_re_encodes() {
    let mut device = device();
    let scene = artwork_page(&mut device);
    let viewport = viewport();
    let mut retained = RetainedScene::new(scene);
    retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "first frame",
    );
    retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Replayed,
        "unchanged",
    );
    device.set_coverage(Coverage::Gpu);
    retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "the lane that makes coverage bytes changed",
    );
}

/// **A released resource**, and the strongest of these tests: the retained encode names
/// outline ids, and a replay of it after the outline is gone would draw a resource this
/// device no longer has. It does not — the frame re-encodes and the encode refuses by
/// name, which is what `Device::render` over the same scene does.
///
/// Principle 6 in one assertion: **the replay did not mask the refusal.**
#[test]
fn a_released_outline_re_encodes_and_the_refusal_stands() {
    let mut device = device();
    let (scene, outlines) = text_page(&mut device, 60, 8, 10.0);
    let viewport = viewport();
    let mut retained = RetainedScene::new(scene);
    retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "first frame",
    );
    retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Replayed,
        "unchanged",
    );

    device.release(outlines[0]).unwrap();
    let refused = device.render_retained(&mut retained, &viewport, Target::Readback);
    assert!(
        matches!(refused, Err(RenderError::UnknownOutline { outline }) if outline == outlines[0]),
        "a frame drawing a released outline must be refused by name, not drawn from a \
         stale instance stream: {refused:?}"
    );
    assert!(
        !retained.holds_encode(),
        "a refused encode leaves nothing retained — the key that was held did not match, \
         so what it held was stale"
    );
    let again = device.render_retained(&mut retained, &viewport, Target::Readback);
    assert!(
        matches!(again, Err(RenderError::UnknownOutline { .. })),
        "and it is refused identically on every later attempt: {again:?}"
    );
}

/// **The atlas generation.** A repack moves every tile, and the retained quad instances
/// carry absolute texel origins into the sheet — so a frame that repacks the atlas
/// invalidates every encode made against the layout it replaced.
///
/// The atlas here is 48×48 texels and every tile is one shape at one phase, so the
/// packer's capacity is arithmetic rather than luck: shelves as tall as the tile, filled
/// left to right. The crowded scene asks for more distinct tiles than fit while asking
/// for fewer *bytes* than the atlas holds — which is exactly ADR 0024's repack condition,
/// and the only shape of frame that resets the atlas.
///
/// Two assertions guard the fixture itself, because a test whose setup silently stops
/// reproducing the condition is a test that proves nothing: the crowded frame must put
/// tiles through to the scratch sheet (`Counters::tiles`), and the frame after it must
/// find the atlas empty (`Counters::atlas_entries`).
#[test]
fn an_atlas_repack_re_encodes() {
    let mut device = device_with(&Options {
        atlas_budget: 48 * 48,
        ..Options::default()
    });
    let (small, _) = text_page(&mut device, 4, 2, 7.0);
    let (crowded, _) = text_page(&mut device, 50, 25, 7.0);
    let viewport = viewport();

    let mut retained = RetainedScene::new(small);
    retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "first frame",
    );
    retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Replayed,
        "unchanged",
    );

    let crowding = device
        .render(&crowded, &viewport, Target::Readback)
        .expect("a page too big for the atlas still draws — through the scratch sheet");
    assert!(
        crowding.counters().tiles > 0,
        "this fixture only tests what it claims to if the atlas actually refused tiles: {:?}",
        crowding.counters()
    );

    let after = retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "the atlas was repacked, so every texel origin the encode names has moved",
    );
    assert_eq!(
        after.counters().atlas_entries,
        2,
        "the repack emptied the atlas, and this frame put its own two tiles back into it"
    );
}

/// **The device.** An encode names atlas positions and resource ids belonging to one
/// device; a handle carried to another one encodes afresh rather than replaying into a
/// texture that device never wrote.
#[test]
fn another_device_re_encodes() {
    let mut first = device();
    let (scene, _) = text_page(&mut first, 60, 8, 10.0);
    let viewport = viewport();
    let mut retained = RetainedScene::new(scene);
    retained_frame(
        &mut first,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "first frame",
    );
    retained_frame(
        &mut first,
        &mut retained,
        &viewport,
        EncodeSource::Replayed,
        "unchanged",
    );

    // The same outline ids, uploaded again on the second device so the scene is
    // drawable there: what is being tested is that the *encode* does not travel.
    let mut second = device();
    let (_, _) = text_page(&mut second, 60, 8, 10.0);
    retained_frame(
        &mut second,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "a retained encode belongs to the device that made it",
    );
}

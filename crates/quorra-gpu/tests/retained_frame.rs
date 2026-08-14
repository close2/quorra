//! A replayed frame is the frame that was encoded, and every way it could stop being
//! one.
//!
//! ADR 0048. `Device::render_retained` skips phase 1 when nothing an encode reads has
//! changed. That is a machine for producing a plausible-looking wrong page — principle
//! 6's worst outcome — so this file is written the way ADR 0045's invalidation list is:
//! **enumerated, one test per entry**, and each asserting the observable
//! `Frame::encode_source` rather than a duration. A test that concluded "the second
//! frame was faster" would pass on a machine where it was faster for another reason,
//! and would say nothing at all about which encode drew the pixels.
//!
//! Four things are checked here and nowhere else:
//!
//! - **byte identity** — a replayed frame's pixels equal a re-encoded frame's, on two
//!   page shapes that take different lanes (the glyph atlas; the coverage sheet, a
//!   curve clip, a blended group and the compositor);
//! - **a page the atlas cannot hold replays like any other** (ADR 0050), which is the
//!   one shape that used to invalidate its own encode on every frame — and the counter
//!   that says whether the atlas settled or is thrashing;
//! - **every entry of the invalidation list** forces a re-encode;
//! - **a refusal is not masked by a replay** — the frame a released outline should
//!   refuse is refused, where a stale encode would have drawn it happily.

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
    Counters, Coverage, Device, EncodeSource, Frame, Options, RenderError, RetainedScene, Target,
    Viewport, wgpu,
};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, GroupSpec, LineCap, LineJoin, OutlineId, Paint,
    Point, Rect, Scene, SceneBuilder, Segment, Stroke,
};

const W: u32 = 220;
const H: u32 = 180;

fn device_with(options: &Options) -> Device {
    let device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..options.clone()
    })
    .expect("llvmpipe is present wherever this suite runs");
    device.wait_until_warm();
    device
}

fn device() -> Device {
    device_with(&Options::default())
}

fn viewport() -> Viewport<'static> {
    Viewport::full(W, H, Affine::IDENTITY)
}

/// A closed curve of `segments` cubics, `side` across — the shape a letterform has for
/// costing purposes, as in `tests/archetypes.rs`.
fn blob(segments: u32, side: f32) -> Vec<Segment> {
    let radius = side * 0.5;
    let mut path = vec![Segment::MoveTo(Point::new(-radius, 0.0))];
    for step in 0..segments.max(3) {
        let angle = |i: u32| (i as f32) / (segments.max(3) as f32) * std::f32::consts::TAU;
        let at = |a: f32| Point::new(radius * a.cos(), radius * a.sin() * 1.3);
        let (a, b) = (at(angle(step)), at(angle(step + 1)));
        path.push(Segment::CubicTo {
            c1: Point::new(a.x + (b.x - a.x) * 0.35, a.y + (b.y - a.y) * 0.1),
            c2: Point::new(a.x + (b.x - a.x) * 0.65, a.y + (b.y - a.y) * 0.9),
            to: b,
        });
    }
    path.push(Segment::Close);
    path
}

fn place(index: u32, side: f32) -> Affine {
    let step = side + 2.0;
    let columns = ((W as f32 - 8.0) / step).max(1.0) as u32;
    Affine::translate(
        6.0 + (index % columns) as f32 * step,
        10.0 + (index / columns) as f32 * step,
    )
}

/// A page of small repeated shapes: the glyph lane, most of it answered by the atlas.
///
/// One `side` for every outline, so every tile the atlas is asked for is the same size —
/// which is what lets `an_atlas_repack_re_encodes` reason about the packer's capacity
/// rather than hope.
fn text_page(
    device: &mut Device,
    placements: u32,
    distinct: u32,
    side: f32,
) -> (Scene, Vec<OutlineId>) {
    let outlines: Vec<OutlineId> = (0..distinct)
        .map(|_| device.upload_outline(&blob(12, side)).unwrap())
        .collect();
    let mut builder = SceneBuilder::new();
    for index in 0..placements {
        builder
            .fill(
                outlines[(index as usize) % outlines.len()],
                place(index, side + 2.0),
                FillRule::NonZero,
                Paint::Solid(Color::new(0.1, 0.12, 0.2, 1.0)),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
    }
    (builder.finish(), outlines)
}

/// The other lanes on one page: strokes, a **curve** clip (so every command under it
/// leaves a coverage tile on the frame's scratch sheet), and a blended group (so the
/// frame allocates layer textures and runs the compositor).
fn artwork_page(device: &mut Device) -> Scene {
    let shape = device.upload_outline(&blob(8, 26.0)).unwrap();
    let clip_shape = device.upload_outline(&blob(6, 120.0)).unwrap();
    let mut builder = SceneBuilder::new();
    let clip = builder
        .clip(
            clip_shape,
            Affine::translate(W as f32 * 0.5, H as f32 * 0.5),
            FillRule::NonZero,
            None,
        )
        .unwrap();
    for index in 0..12 {
        builder
            .stroke(
                shape,
                place(index, 30.0),
                Stroke {
                    width: 2.0,
                    cap: LineCap::Round,
                    join: LineJoin::Miter,
                    miter_limit: 4.0,
                },
                Paint::Solid(Color::new(0.8, 0.2, 0.1, 1.0)),
                Some(clip),
                BlendMode::Normal,
                None,
            )
            .unwrap();
    }
    builder
        .group(
            GroupSpec {
                alpha: 0.7,
                blend: BlendMode::Multiply,
                clip: Some(clip),
                knockout: false,
                mask: None,
                isolated: true,
                compose: Compose::SrcOver,
            },
            |body| {
                for index in 0..6 {
                    body.fill(
                        shape,
                        place(index + 3, 30.0),
                        FillRule::NonZero,
                        Paint::Solid(Color::new(0.1, 0.5, 0.9, 1.0)),
                        Some(clip),
                        BlendMode::Normal,
                        Compose::SrcOver,
                        None,
                    )?;
                }
                Ok(())
            },
        )
        .unwrap();
    builder.finish()
}

fn pixels(frame: Frame) -> Vec<u8> {
    frame.into_raster().unwrap().into_pixels()
}

/// Render through the handle and assert which encode drew it.
fn retained_frame(
    device: &mut Device,
    retained: &mut RetainedScene,
    viewport: &Viewport<'_>,
    expected: EncodeSource,
    why: &str,
) -> Frame {
    let frame = device
        .render_retained(retained, viewport, Target::Readback)
        .expect("the frame must draw");
    assert_eq!(frame.encode_source(), expected, "{why}");
    frame
}

// ---------------------------------------------------------------------------
// (a) Byte identity: a replayed frame is the frame that was encoded.
// ---------------------------------------------------------------------------

/// The three renderings of one scene at one viewport — immediate, retained-and-encoded,
/// retained-and-replayed — are the same bytes.
///
/// On **two page shapes** on purpose: the atlas lane retains nothing but instance bytes
/// and reads tiles the device already holds, while the artwork page's encode carries a
/// coverage sheet, a clip residue, a layer plan and a composite. A replay that dropped
/// the sheet would pass the first and fail the second.
fn identical_across_replay(build: impl Fn(&mut Device) -> Scene, name: &str) {
    let mut device = device();
    let scene = build(&mut device);
    let viewport = viewport();

    let immediate = device
        .render(&scene, &viewport, Target::Readback)
        .expect("the frame must draw");
    assert_eq!(
        immediate.encode_source(),
        EncodeSource::Encoded,
        "{name}: `render` retains nothing and always encodes"
    );
    let immediate = pixels(immediate);

    let mut retained = RetainedScene::new(scene);
    let first = retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "the first frame of a handle has nothing to replay",
    );
    let second = retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Replayed,
        "nothing changed, so the encode is the one already made",
    );
    let third = retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Replayed,
        "a replay does not consume what it replays",
    );

    assert_eq!(
        pixels(first),
        immediate,
        "{name}: the retained path's own first frame must equal an immediate one"
    );
    assert_eq!(
        pixels(second),
        immediate,
        "{name}: a replayed frame must equal the frame that was encoded, byte for byte"
    );
    assert_eq!(
        pixels(third),
        immediate,
        "{name}: and it must still be equal on the frame after that"
    );
}

#[test]
fn a_replayed_text_page_is_the_page_that_was_encoded() {
    identical_across_replay(|device| text_page(device, 240, 20, 10.0).0, "text page");
}

#[test]
fn a_replayed_artwork_page_is_the_page_that_was_encoded() {
    identical_across_replay(artwork_page, "artwork page");
}

// ---------------------------------------------------------------------------
// (a2) A page the atlas cannot hold: the shape that used to replay never (ADR 0050).
// ---------------------------------------------------------------------------

/// The atlas and the page that overflows it, sized so that **the working set fits by
/// bytes and does not fit by packing** — which is the one band in which the old repack
/// rule fired forever.
///
/// A 96×96 atlas holds 9 216 texels and every tile here is one shape at one integer
/// phase — 14×18, or 252 texels — so the packer's capacity is arithmetic rather than
/// luck: six tiles to a shelf (96/14, and 12 texels wasted), five shelves (96/18, and
/// 6 rows wasted), **30 tiles**. The 34 distinct shapes ask for 8 568 texels, which is
/// comfortably inside 9 216 and four tiles more than the shelves hold.
///
/// That gap between the two arithmetics is the whole of the band: the byte test says a
/// repack would fit the working set and the packer says it would not, every frame,
/// forever. The two guard assertions below check the condition rather than trusting
/// these numbers to keep meaning what they mean — the tile size is the rasteriser's
/// answer, not this file's.
const OVERFLOW_ATLAS: u64 = 96 * 96;
const OVERFLOW_DISTINCT: u32 = 34;
const OVERFLOW_PLACEMENTS: u32 = 68;
const OVERFLOW_SIDE: f32 = 13.0;

/// The two conditions that make a frame the one this section is about: a tile went to
/// the scratch sheet, and the frame's own working set would fit the atlas by bytes.
///
/// Asserted at the top of each test, because a fixture that silently stops reproducing
/// its condition is a test that proves nothing — and here it would go on passing, since
/// a page that never overflows replays trivially.
fn assert_overflows_but_fits_by_bytes(counters: &Counters, why: &str) {
    assert!(
        counters.tiles > 0,
        "{why}: this fixture only tests what it claims to if the atlas refused a tile: {counters:?}"
    );
    assert!(
        counters.atlas_working_set_bytes <= OVERFLOW_ATLAS,
        "{why}: and only if the working set fits the atlas by bytes, which is what makes \
         a repack look worth taking: {counters:?}"
    );
}

/// **The headline of ADR 0050.** A page whose glyph tiles overflow the atlas replays
/// like any other page, and draws the pixels a fresh encode draws.
///
/// It did not, and the reason was a loop the frame closed on itself: the tile that fell
/// through to the scratch sheet made the device repack the atlas, the repack bumped the
/// generation the encode had just been stored under, and the next frame re-encoded and
/// did the same again. Twelve frames, twelve encodes, on a page a reader is holding
/// still. Magnified text at a modest atlas budget is that shape.
///
/// Nothing foreign is in this atlas — it is a fresh device — so a repack would re-pack
/// the frame's own tiles in the frame's own encounter order and reproduce the layout it
/// replaced, tile for tile, including the one that overflows. That is why not taking it
/// is not a compromise, and `atlas_repacked` says it happened zero times.
#[test]
fn a_page_the_atlas_cannot_hold_replays_after_its_first_frame() {
    let mut device = device_with(&Options {
        atlas_budget: OVERFLOW_ATLAS,
        ..Options::default()
    });
    let (scene, _) = text_page(
        &mut device,
        OVERFLOW_PLACEMENTS,
        OVERFLOW_DISTINCT,
        OVERFLOW_SIDE,
    );
    let viewport = viewport();
    let mut retained = RetainedScene::new(scene.clone());

    let first = retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "the first frame of a handle has nothing to replay",
    );
    assert_overflows_but_fits_by_bytes(&first.counters(), "the first frame");
    assert!(
        !first.counters().atlas_repacked,
        "an atlas holding nothing but this frame's own tiles repacks to the layout it \
         already has: {:?}",
        first.counters()
    );
    let first = pixels(first);

    for round in 0..4 {
        let frame = retained_frame(
            &mut device,
            &mut retained,
            &viewport,
            EncodeSource::Replayed,
            "an overflowing page is still an unchanged page",
        );
        assert!(
            !frame.counters().atlas_repacked,
            "round {round}: a replay inserts nothing and so settles nothing: {:?}",
            frame.counters()
        );
        assert_eq!(
            pixels(frame),
            first,
            "round {round}: the replayed page must be the page that was encoded"
        );
    }

    // The other direction, and the one that would catch a replay drawing from a moved
    // atlas: a frame encoded from scratch *now*, against the atlas as it stands.
    let fresh = device
        .render(&scene, &viewport, Target::Readback)
        .expect("the page draws");
    assert_eq!(
        fresh.encode_source(),
        EncodeSource::Encoded,
        "`render` retains nothing and always encodes"
    );
    assert!(
        !fresh.counters().atlas_repacked,
        "and it finds nothing foreign to reclaim either: {:?}",
        fresh.counters()
    );
    assert_eq!(
        pixels(fresh),
        first,
        "a freshly encoded frame and a replayed one are the same page, to the byte"
    );
}

/// **The oscillation counter.** An atlas holding another page's tiles repacks **once**,
/// and the frame after it finds nothing left to reclaim.
///
/// This is the sequence the old rule could not end: repack, re-encode, overflow, repack.
/// `Counters::atlas_repacked` is what makes the difference between the two behaviours
/// observable from outside — a page that settles reports it true on one frame and false
/// on every later one, and a page that thrashes would report it true forever.
///
/// Two encodes and then replays for ever is the *most* a page can cost here, and that
/// bound is the decision: the first frame pays for the atlas it inherited, the second
/// pays for the layout that replaced it, and the third onwards pay nothing.
#[test]
fn an_atlas_holding_another_page_repacks_once_and_then_settles() {
    let mut device = device_with(&Options {
        atlas_budget: OVERFLOW_ATLAS,
        ..Options::default()
    });
    // A different page first, so the atlas holds tiles the page below will never ask
    // for. Its outlines are its own, so no key of it can collide with one of theirs.
    let (foreign, _) = text_page(&mut device, 6, 3, OVERFLOW_SIDE);
    let (crowded, _) = text_page(
        &mut device,
        OVERFLOW_PLACEMENTS,
        OVERFLOW_DISTINCT,
        OVERFLOW_SIDE,
    );
    let viewport = viewport();
    device
        .render(&foreign, &viewport, Target::Readback)
        .expect("the small page draws");

    let mut retained = RetainedScene::new(crowded);
    let first = retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "the first frame of a handle has nothing to replay",
    );
    assert_overflows_but_fits_by_bytes(&first.counters(), "the first frame");
    assert!(
        first.counters().atlas_repacked,
        "the atlas held another page's tiles and this frame had no room: reclaiming them \
         is the one thing a repack does: {:?}",
        first.counters()
    );

    let second = retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "the repack moved every texel origin the first encode named",
    );
    assert!(
        !second.counters().atlas_repacked,
        "and now there is nothing foreign left, so a second repack would reproduce this \
         very layout: {:?}",
        second.counters()
    );

    let mut repacks = 0_u32;
    for round in 0..6 {
        let frame = retained_frame(
            &mut device,
            &mut retained,
            &viewport,
            EncodeSource::Replayed,
            "the atlas has settled, so the encode made against it still holds",
        );
        repacks += u32::from(frame.counters().atlas_repacked);
        assert_eq!(repacks, 0, "round {round}: the atlas repacked again");
    }
}

/// The counters of a replayed frame are the counters of the frame it replays — **except
/// the one that is about this frame's transfers rather than about the encode**.
///
/// `bytes_uploaded` is honestly smaller on a replay, and by a knowable amount: the frame
/// that encoded also uploaded the glyph tiles it rasterised into the atlas texture, and
/// the frame that replayed uploaded only the instance bytes, because the tiles are
/// already resident. That is a counter telling the truth about two different frames, and
/// it is why this test compares the encode-derived fields by equality and that one by
/// inequality rather than blurring the two.
#[test]
fn a_replay_counts_what_the_encode_counted() {
    let mut device = device();
    let (scene, _) = text_page(&mut device, 240, 20, 10.0);
    let viewport = viewport();
    let mut retained = RetainedScene::new(scene);
    let first = retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "first frame",
    );
    let second = retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Replayed,
        "second frame",
    );
    let (first, second) = (first.counters(), second.counters());
    assert_eq!(
        Counters {
            bytes_uploaded: 0,
            ..first
        },
        Counters {
            bytes_uploaded: 0,
            ..second
        },
        "everything a counter reads out of the encode must be the encode's"
    );
    assert!(
        second.bytes_uploaded < first.bytes_uploaded,
        "the replay uploaded its instances and no glyph tiles: {} against {}",
        second.bytes_uploaded,
        first.bytes_uploaded
    );
}

/// A replayed frame's **encode subdivision is zero, not the retained encode's**
/// (ADR 0023's instrument, ADR 0048's honesty).
///
/// The clock lives inside the `Encoded`, so the geometry and staging totals a retained
/// encode carries are real durations spent by the frame that made it — possibly hundreds
/// of frames ago. A replay that reported them would be a `Frame` saying it spent time it
/// did not spend, which is the one thing a `Frame` may never do.
#[test]
fn a_replay_reports_no_geometry_and_no_staging() {
    let mut device = device_with(&Options {
        instrument_encode: true,
        ..Options::default()
    });
    let scene = artwork_page(&mut device);
    let viewport = viewport();
    let mut retained = RetainedScene::new(scene);

    let phase = |frame: &Frame, name: &str| {
        frame
            .timings()
            .phases
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, d)| *d)
    };

    let first = retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "first frame",
    );
    assert!(
        phase(&first, "encode: geometry").is_some_and(|d| d > std::time::Duration::ZERO),
        "the encoding frame rasterised this page's coverage and must say so: {:?}",
        first.timings().phases
    );

    let second = retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Replayed,
        "second frame",
    );
    for name in ["encode: geometry", "encode: staging", "encode: recording"] {
        assert_eq!(
            phase(&second, name),
            Some(std::time::Duration::ZERO),
            "a replayed frame spent nothing on {name}: {:?}",
            second.timings().phases
        );
    }
}

// ---------------------------------------------------------------------------
// (b) The invalidation list, one test per entry (ADR 0045).
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// (c) Refusals, and (d) the caller who ignores all of this.
// ---------------------------------------------------------------------------

/// A scene refused at encode is refused every time, and retains nothing to replay: the
/// budget check is the first thing phase 1 does, so there is no window in which a
/// refusing frame leaves an encode behind.
#[test]
fn a_refused_scene_refuses_identically_on_every_attempt() {
    let mut device = device_with(&Options {
        max_frame_bytes: 64,
        ..Options::default()
    });
    let (scene, _) = text_page(&mut device, 60, 8, 10.0);
    let viewport = viewport();
    let mut retained = RetainedScene::new(scene);
    for attempt in 0..3 {
        let refused = device.render_retained(&mut retained, &viewport, Target::Readback);
        assert!(
            matches!(refused, Err(RenderError::FrameBudgetExceeded { .. })),
            "attempt {attempt}: {refused:?}"
        );
        assert!(
            !retained.holds_encode(),
            "attempt {attempt}: a refused frame retains nothing"
        );
        assert_eq!(retained.retained_bytes(), 0);
    }
}

/// A viewport refused before phase 1 is refused whether or not an encode is held: the
/// validation runs on every call, so a replay cannot carry a frame past a check that
/// `Device::render` would have failed.
#[test]
fn a_refused_viewport_is_refused_with_an_encode_in_hand() {
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
    assert!(retained.holds_encode());

    let broken = Viewport::full(W, H, Affine::translate(f32::NAN, 0.0));
    let refused = device.render_retained(&mut retained, &broken, Target::Readback);
    assert!(
        matches!(refused, Err(RenderError::NonFiniteViewportTransform)),
        "{refused:?}"
    );
    retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Replayed,
        "the refusal touched nothing: the held encode is still the one for this viewport",
    );
}

/// The handle can be built on one thread and rendered from another: `Send`, and the
/// scene it holds is still the `Send + Sync` scene the brief's §2.3 asks for.
#[test]
fn the_handle_travels_between_threads() {
    fn assert_send<T: Send>() {}
    fn assert_send_sync<T: Send + Sync + 'static>() {}
    assert_send::<RetainedScene>();
    assert_send_sync::<Scene>();

    let mut device = device();
    let (scene, _) = text_page(&mut device, 12, 4, 10.0);
    let retained = std::thread::spawn(move || RetainedScene::new(scene))
        .join()
        .expect("building a handle cannot fail");
    let mut retained = retained;
    retained_frame(
        &mut device,
        &mut retained,
        &viewport(),
        EncodeSource::Encoded,
        "a handle built elsewhere draws here",
    );
}

/// A blank scene is a legitimate scene, and a legitimate thing to retain (§5).
#[test]
fn a_blank_scene_replays_like_any_other() {
    let mut device = device();
    let viewport = viewport();
    let mut retained = RetainedScene::new(SceneBuilder::new().finish());
    let first = retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "first frame",
    );
    let second = retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Replayed,
        "a blank scene is a scene",
    );
    assert_eq!(first.counters().commands, 0);
    assert_eq!(pixels(first), pixels(second));
}

/// What the handle costs is a number the caller can read, and `forget` gives it back.
#[test]
fn the_retained_bytes_are_reported_and_returnable() {
    let mut device = device();
    let scene = artwork_page(&mut device);
    let viewport = viewport();
    let mut retained = RetainedScene::new(scene);
    assert_eq!(
        retained.retained_bytes(),
        0,
        "a handle that has never drawn holds no encode"
    );
    retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "first frame",
    );
    assert!(
        retained.retained_bytes() > 0,
        "an artwork page's encode is instance bytes, a coverage sheet and a plan tree"
    );
    retained.forget();
    assert_eq!(retained.retained_bytes(), 0);
    assert!(!retained.holds_encode());
    retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "forgetting costs exactly one encode",
    );
}

/// **The caller who ignores all of this sees no change.** `Device::render` retains
/// nothing, replays nothing and reports `Encoded` on every frame — including frames of
/// a scene it has just drawn, which is today's behaviour and stays it.
#[test]
fn render_retains_nothing() {
    let mut device = device();
    let (scene, _) = text_page(&mut device, 60, 8, 10.0);
    let viewport = viewport();
    let mut before = None;
    for _ in 0..3 {
        let frame = device
            .render(&scene, &viewport, Target::Readback)
            .expect("the frame must draw");
        assert_eq!(frame.encode_source(), EncodeSource::Encoded);
        let bytes = pixels(frame);
        if let Some(previous) = before.replace(bytes.clone()) {
            assert_eq!(previous, bytes, "an unchanged scene draws unchanged pixels");
        }
    }
}

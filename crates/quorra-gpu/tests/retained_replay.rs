//! A replayed frame is the frame that was encoded — in its pixels, in its counters, and
//! in what it says it spent.
//!
//! ADR 0048. `Device::render_retained` skips phase 1 when nothing an encode reads has
//! changed, which is a machine for producing a plausible-looking wrong page — principle
//! 6's worst outcome. This file is the equality half of the answer: the bytes a replay
//! draws are the bytes an encode drew, and everything the replayed `Frame` reports about
//! itself is true of *that* frame rather than of the one it replays. What makes an encode
//! stop being replayable is `retained_invalidation.rs`; what a page too large for the
//! atlas does is `retained_atlas.rs`.
//!
//! Every assertion here is an observable — `Frame::encode_source`, a raster, a counter, a
//! phase — and never a duration. A test that concluded "the second frame was faster"
//! would pass on a machine where it was faster for another reason, and would say nothing
//! at all about which encode drew the pixels.

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

use quorra_gpu::{Counters, Device, EncodeSource, Frame, Options, RetainedScene, Target};
use quorra_scene::Scene;

mod common;

use common::headless::pixels;
use common::retained::{artwork_page, device, device_with, retained_frame, text_page, viewport};

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

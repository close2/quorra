//! What `RetainedScene` promises about itself, and what the caller who ignores it sees.
//!
//! ADR 0048 added a handle, and a handle is API before it is an optimisation: it travels
//! between threads, it holds a scene that is still `Send + Sync`, it says what it costs
//! and gives it back, and it accepts a blank scene like any other (principle 6 — a blank
//! scene is a legitimate scene). None of that is about *which* encode drew a frame, which
//! is why none of it is in `retained_replay.rs` or `retained_invalidation.rs`.
//!
//! The last test is the one that says the feature is opt-in: a caller still using
//! `Device::render` gets today's behaviour unchanged, on every frame, for ever.

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

use quorra_gpu::{EncodeSource, RetainedScene, Target};
use quorra_scene::{Scene, SceneBuilder};

mod common;

use common::headless::pixels;
use common::retained::{artwork_page, device, retained_frame, text_page, viewport};

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

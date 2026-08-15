//! A refusal is not masked by a replay.
//!
//! Principle 6: a frame is drawn, or it is refused, and the retained path adds a way for
//! the second to turn quietly into a third state. A held encode is a frame that already
//! passed every check once, so the danger is a call that reaches it without running them
//! again — and the two checks that bracket phase 1 are the ones tested here: the frame
//! budget, which phase 1 does first, and the viewport validation, which runs before phase
//! 1 is reached at all.
//!
//! The third refusal of the retained path — a released outline, where the *encode* is what
//! goes stale — is an entry of the invalidation list and lives in
//! `retained_invalidation.rs` with the rest of it.

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

use quorra_gpu::{EncodeSource, Options, RenderError, RetainedScene, Target, Viewport};
use quorra_scene::Affine;

mod common;

use common::retained::{H, W, device, device_with, retained_frame, text_page, viewport};

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

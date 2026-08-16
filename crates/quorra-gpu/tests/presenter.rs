//! What can be settled about the presenter **without a window**: the threading claim it
//! is built on, and what a device without a surface answers when asked for one.
//!
//! Everything that needs a real swapchain — detaching, presenting under an affine,
//! attaching back, and `Target::Surface` refusing by name in between — is
//! `examples/present_thread.rs`, which runs under `Xvfb` in CI and reads the window's
//! pixels back with `xwd`. This file is the half that a headless test suite can carry,
//! and the split is deliberate: nothing here opens a display, so the corpus gate, the
//! oracle and every golden in this tree stay as unaware of the presenter as they are of
//! the surface (ADR 0056's determinism clause).

// Integration-test files sit outside `#[cfg(test)]`, so `clippy.toml`'s
// allow-panic-in-tests does not reach them; this is the same policy, stated here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use quorra_gpu::{Device, Layer, Options, PresentCost, Presenter, RenderError, Target, Viewport};
use quorra_scene::{Affine, Scene, SceneBuilder};

fn headless() -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

/// **The claim the whole split rests on, asserted by the compiler.**
///
/// A `Presenter` is `Send` because everything it holds is: `wgpu::Surface<'static>`,
/// `Device`, `Queue` and `Sampler` are each `Send + Sync` on native targets — `wgpu`
/// asserts exactly that in its own source, under a `send_sync` cfg that is
/// `not(target_arch = "wasm32")` — and the pipeline store behind the `Arc` was already
/// shared with the warm-up thread, so it was already both. This crate is
/// `#![forbid(unsafe_code)]`, so no `unsafe impl` can be hiding behind the assertion:
/// if it holds, it holds by construction.
///
/// The same shape as `tests/retained_handle.rs`'s assertion for `RetainedScene`, and
/// for the same reason — a threading promise nothing checks is a promise that breaks on
/// the day a field is added.
#[test]
fn a_presenter_is_send_and_a_layer_crosses_threads_with_it() {
    fn assert_send<T: Send>() {}
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send::<Presenter>();
    // A layer is a borrow of a texture plus two values; `wgpu::Texture` is `Send + Sync`
    // too, which is what lets a host render into one on its render thread and present it
    // from its event thread.
    assert_send_sync::<Layer<'_>>();
    assert_send_sync::<PresentCost>();
    // And the device itself, which in the caller's arrangement is what moves: the
    // presenter stays on the thread that owns the window.
    assert_send::<Device>();
}

/// A headless device has no surface to hand over, and says so by handing over nothing.
#[test]
fn a_headless_device_has_no_presenter_to_detach() {
    let mut device = headless();
    assert!(device.detach_presenter().is_none());
    // And is unchanged by having been asked: the second call is not a different
    // question, and neither call turned this device into a detached one.
    assert!(device.detach_presenter().is_none());
}

/// The refusals a device without a surface gives are the ones it always gave. A
/// headless device is **not** a detached one, and the two errors are different words
/// on purpose — this is the regression that a third surface state must not introduce.
#[test]
fn a_headless_device_still_refuses_a_surface_target_as_headless() {
    let mut device = headless();
    let scene: Scene = SceneBuilder::new().finish();
    let viewport = Viewport::full(64, 64, Affine::IDENTITY);
    match device.render(&scene, &viewport, Target::Surface) {
        Err(RenderError::NoSurface) => {}
        other => panic!("expected NoSurface on a headless device, got {other:?}"),
    }
    match device.invalidate_surface() {
        Err(RenderError::NoSurface) => {}
        other => panic!("expected NoSurface from invalidate_surface, got {other:?}"),
    }
    // Asking for a presenter first changes nothing: there was none to take.
    assert!(device.detach_presenter().is_none());
    match device.render(&scene, &viewport, Target::Surface) {
        Err(RenderError::NoSurface) => {}
        other => panic!("a headless device stays headless, got {other:?}"),
    }
}

/// A headless device renders exactly as it did — the point being that the presenter
/// exists and the two target kinds a golden file uses cannot see it.
#[test]
fn the_targets_a_golden_uses_are_untouched_by_the_split() {
    let mut device = headless();
    let scene: Scene = SceneBuilder::new().finish();
    let viewport = Viewport::full(16, 8, Affine::IDENTITY);
    let frame = device
        .render(&scene, &viewport, Target::Readback)
        .expect("a blank scene is a legitimate scene");
    let raster = frame.into_raster().expect("a readback frame carries one");
    assert_eq!(raster.width(), 16);
    assert_eq!(raster.height(), 8);
    assert!(
        raster.pixels().iter().all(|byte| *byte == 0),
        "a page that marks nothing is transparent (§3)"
    );
}

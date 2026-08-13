//! A device does not outlive its own background thread.
//!
//! ADR 0018. A `Device` compiles its warm pipeline set on a thread nobody blocks on,
//! and until that ADR the thread was detached — so a host that built devices and
//! dropped them without rendering (probing adapters is exactly that) could reach
//! `exit()` with a thread still inside the driver, and the driver's own atexit teardown
//! then crashed the process. Measured on this machine before the fix: **11 of 12 runs**
//! of the churn below died with SIGSEGV or SIGABRT after every test had passed, the
//! faulting thread named `quorra-warm-up`.
//!
//! **The assertion is the exit status of this binary**, which is why the tests look
//! like they check nothing: a crash in teardown happens after the last test reports,
//! and cargo fails the run on the signal. The churn is split across several tests
//! rather than looped inside one because the crash needed threads racing — one test
//! constructing while another's device tore down — and the harness's parallelism is
//! what supplies that.

// Test-file lint policy as in m1.rs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use quorra_gpu::{Device, Options, Target, Viewport, WarmUp};
use quorra_scene::{Affine, BlendMode, Color, Compose, GroupSpec, Point, Rect, SceneBuilder};

/// Build a device on every adapter this machine has and drop each one at once —
/// before the warm-up thread it started can plausibly have finished.
fn churn() {
    let names = Device::adapter_names();
    assert!(
        !names.is_empty(),
        "no adapter at all: nothing was exercised"
    );
    for name in names {
        // An adapter that cannot yield a device (a GL adapter this suite does not use)
        // is not this test's subject; it is skipped rather than failed.
        if let Ok(device) = Device::headless(&Options {
            adapter: Some(name),
            ..Options::default()
        }) {
            assert!(!device.description().is_empty());
        }
    }
}

#[test]
fn churn_a() {
    churn();
}

#[test]
fn churn_b() {
    churn();
}

#[test]
fn churn_c() {
    churn();
}

#[test]
fn churn_d() {
    churn();
}

/// The other order, for the same reason: a device that *is* warm before it is dropped
/// must still be droppable, and joining an already-finished thread must not hang.
#[test]
fn a_warm_device_drops_too() {
    let device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs");
    device.wait_until_warm();
    assert!(device.is_warm());
    // The warm set is real, so the state carries the duration it cost — the outcome a
    // caller polling `warm_up` distinguishes from a refusal that will never become one
    // (ADR 0042). `is_warm` is the same question with that distinction thrown away.
    let WarmUp::Warm(compiled) = device.warm_up() else {
        panic!("a device that reports itself warm has compiled the warm set");
    };
    assert_eq!(device.startup().pipeline_compilation, Some(compiled));
    drop(device);
}

/// **A warmed device draws the same frame as a cold one** (ADR 0035).
///
/// `Device::warm_for` exists to move an allocation off the first frame, and the property
/// that makes it safe to call — or not to call — is that it changes nothing else: the
/// same scene at the same size gives the same bytes, whether it was warmed for that size,
/// for another one, or not at all. A hint that changed a picture would be worse than the
/// milliseconds it saves.
#[test]
fn warming_changes_the_timing_and_nothing_else() {
    let scene = {
        let mut builder = SceneBuilder::new();
        builder
            .group(
                GroupSpec {
                    alpha: 0.6,
                    blend: BlendMode::Normal,
                    clip: None,
                    knockout: false,
                    mask: None,
                    isolated: true,
                    compose: Compose::SrcOver,
                },
                |body| {
                    body.rect(
                        Rect::new(Point::new(4.0, 4.0), Point::new(48.0, 40.0)),
                        Affine::IDENTITY,
                        Color::new(0.2, 0.5, 0.9, 0.8),
                        None,
                        None,
                    )
                },
            )
            .expect("a group of one rect");
        builder.finish()
    };
    let viewport = Viewport::full(64, 48, Affine::IDENTITY);
    let draw = |warm: Option<(u32, u32)>| {
        let mut device = Device::headless(&Options {
            adapter: Some("llvmpipe".into()),
            ..Options::default()
        })
        .expect("llvmpipe is present wherever this suite runs");
        device.wait_until_warm();
        if let Some((w, h)) = warm {
            device.warm_for(w, h);
        }
        device
            .render(&scene, &viewport, Target::Readback)
            .expect("a group draws")
            .into_raster()
            .unwrap()
            .into_pixels()
    };

    let cold = draw(None);
    assert_eq!(draw(Some((64, 48))), cold, "warmed for this size");
    assert_eq!(draw(Some((800, 600))), cold, "warmed for another size");
    assert_eq!(
        draw(Some((0, 0))),
        cold,
        "a zero size is a no-op, not a panic"
    );
}

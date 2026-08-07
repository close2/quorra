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

use quorra_gpu::{Device, Options};

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
    drop(device);
}

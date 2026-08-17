//! What the store and its warm-up promise, asked through them rather than of them.
//!
//! Two properties, and both are about a refusal rather than a picture: **a pipeline this
//! adapter cannot build is an `Err` that names it** (§5, ADR 0042), and **a warm-up that
//! ends always says so**, including by panicking — the defect that used to hang the whole
//! suite on a `Condvar` with no notifier left alive, which is why every wait here is
//! bounded and runs on a thread of its own.
//!
//! One file for both halves of a module that is two files, deliberately: `--list` names a
//! test by its module path, so dividing these along the source's seam would rename every
//! one of them, and a rename is a change to the gate rather than to what is gated. It is
//! worth doing as its own round, with the renames as the only visible change.
#![allow(clippy::expect_used, clippy::panic)] // test-file policy: a fixture that cannot run must fail loudly

use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::warm::WarmUpGuard;
use super::{Kind, PipelineStore, WARM_FORMAT};
use crate::device::Device;
use crate::error::PipelineProblem;
use crate::startup::{Options, WarmUp};

/// The software adapter, as everywhere in this crate's tests.
fn device() -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

/// **How long a wait is allowed to be before it counts as never ending.** Every
/// test here that could hang runs its wait on a thread and gives up after this, so
/// a regression fails the suite instead of wedging it — which is the trap the
/// defect these tests guard against actually sprang.
const PATIENCE: Duration = Duration::from_secs(30);

/// Call `wait` on a thread and insist it returns. Panics with `what` if it does
/// not, leaving the waiting thread behind: it is blocked on a `Condvar` and holds
/// nothing, so the process still exits.
fn within_patience<T: Send + 'static>(what: &str, wait: impl FnOnce() -> T + Send + 'static) {
    let (done, waited) = mpsc::channel();
    thread::spawn(move || {
        drop(wait());
        let _ = done.send(());
    });
    assert!(
        waited.recv_timeout(PATIENCE).is_ok(),
        "{what} did not return within {PATIENCE:?}"
    );
}

/// The defect ADR 0042 closes, stated as the property that failed: a warm-up that
/// ends by panicking must still release everything waiting on it. Before the guard,
/// this test hung — which is the whole reason the wait is bounded.
#[test]
fn a_warm_up_that_panics_still_releases_its_waiters() {
    let device = device();
    let (gpu, _) = device.wgpu();
    let store = PipelineStore::new(gpu.clone());
    let panicking = {
        let store = Arc::clone(&store);
        thread::spawn(move || {
            let _guard = WarmUpGuard::new(&store);
            panic!("a driver error wgpu reports by panicking rather than by an error scope");
        })
    };
    assert!(panicking.join().is_err(), "the thread panicked as staged");
    within_patience("wait_until_warm after a panicking warm-up", {
        let store = Arc::clone(&store);
        move || store.wait_until_warm()
    });
    assert_eq!(store.warm_up(), WarmUp::Abandoned);
    assert!(store.warm_duration().is_none());
}

/// A store whose warm-up has not run yet keeps `wait_until_warm` waiting — the
/// other half of the property above, so that "it returns" is not passing by
/// returning always.
#[test]
fn a_running_warm_up_is_reported_as_running() {
    let device = device();
    let (gpu, _) = device.wgpu();
    let store = PipelineStore::new(gpu.clone());
    assert_eq!(store.warm_up(), WarmUp::Running);
}

/// A pipeline this adapter cannot build is an `Err` naming it, not a panic on
/// whichever thread asked (§5: refused, never survived).
///
/// `Rgba8Snorm` is the instrument: WebGPU gives it no `RENDER_ATTACHMENT` usage, so
/// a colour target in that format is a validation error every backend agrees on,
/// reached without shipping a shader that does not parse.
#[test]
fn a_pipeline_the_adapter_refuses_is_an_error_and_not_a_panic() {
    let device = device();
    let (gpu, _) = device.wgpu();
    let store = PipelineStore::new(gpu.clone());
    let refused = store.get(Kind::Blit, wgpu::TextureFormat::Rgba8Snorm);
    match refused {
        Err(PipelineProblem::Pipeline {
            pipeline, format, ..
        }) => {
            assert_eq!(pipeline, "quorra blit");
            assert_eq!(format, wgpu::TextureFormat::Rgba8Snorm);
        }
        other => panic!("expected a named pipeline refusal, got {other:?}"),
    }
    // Nothing was cached for the format that failed, and the store still works:
    // one refused format must not take the device with it.
    store
        .get(Kind::Blit, WARM_FORMAT)
        .expect("the blit pipeline builds for the format the frame actually uses");
}

/// A device made for a surface warms the presenting lanes in the surface's own
/// format (ADR 0043), so its first frame does not compile them inside itself —
/// asserted at the store, because this account has no window to present to.
/// `Composite` deliberately stays out of the second set: a composite's target is
/// always an internal accumulator (ADR 0038's hand-off), so warming it for the
/// surface's format would compile a pipeline no frame can reach.
#[test]
fn the_warm_set_includes_the_present_format_when_given_one() {
    let device = device();
    let (gpu, _) = device.wgpu();
    let store = PipelineStore::new(gpu.clone());
    let present = wgpu::TextureFormat::Bgra8Unorm;
    store.warm_up_now(Some(present));
    assert!(matches!(store.warm_up(), WarmUp::Warm(_)));
    let state = store.lock();
    for kind in [Kind::RectOver, Kind::CoverOver, Kind::Blit, Kind::Present] {
        assert!(
            state.pipelines.contains_key(&(kind, present)),
            "{kind:?} missing for the present format"
        );
    }
    assert!(
        !state.pipelines.contains_key(&(Kind::Composite, present)),
        "a composite never targets the surface, so warming it there is waste"
    );
    assert!(
        !state.pipelines.contains_key(&(Kind::Present, WARM_FORMAT)),
        "a presenter draws onto the surface and onto nothing else"
    );
}

/// [`Kind::Present`] is the one member of the second set that is compiled even when
/// the surface negotiated [`WARM_FORMAT`] itself — the other three are already
/// built, and this one has no first set to be in (ADR 0056). This is what makes
/// `Device::detach_presenter` a move rather than a compile: the pipeline a presenter
/// needs exists before it is asked for, built by the thread nobody blocks on.
#[test]
fn the_presenting_pass_is_warmed_for_the_surfaces_format_whatever_it_is() {
    let device = device();
    let (gpu, _) = device.wgpu();
    for present in [WARM_FORMAT, wgpu::TextureFormat::Bgra8Unorm] {
        let store = PipelineStore::new(gpu.clone());
        store.warm_up_now(Some(present));
        assert!(matches!(store.warm_up(), WarmUp::Warm(_)));
        assert!(
            store
                .lock()
                .pipelines
                .contains_key(&(Kind::Present, present)),
            "the present pass is missing for {present:?}"
        );
    }
    // And a headless device warms none of it: there is no surface to present to.
    let store = PipelineStore::new(gpu.clone());
    store.warm_up_now(None);
    assert!(
        !store
            .lock()
            .pipelines
            .keys()
            .any(|(kind, _)| *kind == Kind::Present),
        "a headless device has no surface, so the presenting pass is waste"
    );
}

/// The other half of ADR 0043's rule, which the presenter relies on: a pipeline the
/// warm set did not build is built by the first ask, **inline**, and that ask says
/// what it cost — which is how `PresentCost::compiled` can be truthful about a
/// presenter detached before its device was warm.
#[test]
fn a_pipeline_the_warm_set_missed_is_compiled_by_the_first_ask_and_says_so() {
    let device = device();
    let (gpu, _) = device.wgpu();
    let store = PipelineStore::new(gpu.clone());
    let (_, compiled) = store
        .get(Kind::Present, WARM_FORMAT)
        .expect("the present pipeline builds for a format the adapter renders to");
    assert!(
        compiled.is_some(),
        "the ask that compiled a pipeline has to be able to say so"
    );
    let (_, again) = store
        .get(Kind::Present, WARM_FORMAT)
        .expect("and the second ask finds it cached");
    assert!(
        again.is_none(),
        "a cached pipeline costs nothing and must not claim a compile"
    );
}

/// The other half: a headless device (no present format) and a surface whose
/// format is already [`WARM_FORMAT`] compile no second set at all.
#[test]
fn the_warm_set_compiles_one_format_when_no_second_is_needed() {
    let device = device();
    let (gpu, _) = device.wgpu();
    for present in [None, Some(WARM_FORMAT)] {
        let store = PipelineStore::new(gpu.clone());
        store.warm_up_now(present);
        assert!(matches!(store.warm_up(), WarmUp::Warm(_)));
        let state = store.lock();
        assert!(
            state
                .pipelines
                .keys()
                .all(|(_, format)| *format == WARM_FORMAT),
            "no pipeline outside {WARM_FORMAT:?} belongs in this warm set"
        );
    }
}

/// The refusal above must reach a caller in the words of the error, not only as a
/// variant — a person reading a log has to be able to attribute it.
#[test]
fn a_refusal_names_the_pipeline_and_the_format() {
    let device = device();
    let (gpu, _) = device.wgpu();
    let store = PipelineStore::new(gpu.clone());
    let Err(reason) = store.get(Kind::Composite, wgpu::TextureFormat::Rgba8Snorm) else {
        panic!("Rgba8Snorm is not a renderable format");
    };
    let message = reason.to_string();
    assert!(message.contains("quorra composite"), "{message}");
    assert!(message.contains("Rgba8Snorm"), "{message}");
}

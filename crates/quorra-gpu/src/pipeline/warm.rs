//! The warm-up: what a device compiles before anyone asks, on a thread nobody blocks
//! on, and how it reports what became of it.
//!
//! One thing: a state machine with four states and exactly one writer. [`WarmUp`] runs,
//! then becomes `Warm`, `Refused` or `Abandoned`, once, and never moves again — and the
//! whole of the machinery that has to make that true lives here: the thread that does
//! the compiling, the guard that records an outcome on **every** exit path including an
//! unwind, and the two ways a caller observes the result. `PipelineStore`'s `warmed`
//! condition variable has exactly one notifier and it is in this file, which is the
//! invariant ADR 0042 exists to hold and the reason these belong together rather than
//! beside the compile they call.
//!
//! It compiles nothing itself. Every pipeline here is asked for through
//! [`PipelineStore::get`](super::PipelineStore::get), which is the store's business and
//! stays in its own file; **which** pipelines, and in which formats, is this file's — and
//! that question is a startup-latency argument (ADR 0040, ADR 0043, ADR 0056), not a
//! rendering one.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::error::PipelineProblem;
use crate::startup::WarmUp;

use super::{Kind, PipelineStore, WARM_FORMAT};

/// Records the warm-up's outcome and releases its waiters on **every** exit path,
/// including an unwind.
///
/// [`PipelineStore::wait_until_warm`] is a `Condvar` with exactly one notifier, so a
/// notifier that leaves by a route which does not notify leaves every waiter waiting
/// forever. That is not a hypothetical: a reserved keyword in `blit.wgsl` panicked this
/// thread inside `wgpu`, and the test binary then sat silent until it was killed — the
/// defect ADR 0042 exists to close. The error scopes in
/// [`captured`](super::captured) make the *known* panic a value; this guard is what
/// makes the promise hold for the ones that are not.
pub(super) struct WarmUpGuard<'a> {
    store: &'a PipelineStore,
    /// What to record. Still `None` when the thread is unwinding, which is exactly
    /// [`WarmUp::Abandoned`] — the state with no reason to give, because there is none.
    outcome: Option<WarmUp>,
}

impl<'a> WarmUpGuard<'a> {
    pub(super) fn new(store: &'a PipelineStore) -> Self {
        Self {
            store,
            outcome: None,
        }
    }

    /// Record `outcome` and release the waiters, by dropping.
    fn finish(mut self, outcome: WarmUp) {
        self.outcome = Some(outcome);
    }
}

impl Drop for WarmUpGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.store.lock();
        state.warm_up = self.outcome.take().unwrap_or(WarmUp::Abandoned);
        drop(state);
        self.store.warmed.notify_all();
    }
}

impl PipelineStore {
    /// Compile the warm set on a background thread, so device construction returns
    /// first. §2.1 of the brief also says a device must not *require* a background
    /// thread: if the host cannot spawn one, the warm set compiles inline instead —
    /// construction blocks for the compile, which is the documented cost of a host
    /// with no threads to give, and `warm_up` keeps meaning what it says.
    ///
    /// The handle goes to the [`Device`], which joins it when it is dropped. Nothing
    /// *waits* on it — completion is observed through `warm_up`/`wait_until_warm`, and
    /// a frame that arrives early compiles what it needs on the spot — but a thread
    /// inside the driver must not outlive the device it is compiling for (ADR 0018).
    ///
    /// [`Device`]: crate::device::Device
    pub(crate) fn spawn_warm_up(
        self: &Arc<Self>,
        present_format: Option<wgpu::TextureFormat>,
    ) -> Option<thread::JoinHandle<()>> {
        let store = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("quorra-warm-up".into())
            .spawn(move || store.warm_up_now(present_format));
        let Ok(handle) = spawned else {
            self.warm_up_now(present_format);
            return None;
        };
        Some(handle)
    }

    /// The warm-up itself: the two over-lanes a page of text needs, the two passes
    /// a page with a group needs (§7 — the knockout variants, the reduction and the
    /// winding lane still compile on first use), and — for a device constructed for a
    /// surface — the presenting lanes again in the surface's own format.
    ///
    /// **Why the surface's format is a second set** (ADR 0043): every pipeline is
    /// keyed by `(kind, target format)`, the warm set compiles [`WARM_FORMAT`], and a
    /// surface negotiates `Bgra8Unorm` where the adapter offers it — so a presenting
    /// host's first frame compiled the lane it drew with *inside that frame*,
    /// measured on RADV at 0.3–1.0 ms per fresh device, one entry every time, flat or
    /// layered (`examples/surface_measure.rs`). [`Kind::Composite`] is deliberately
    /// not in the second set: a composite's target is always an internal accumulator
    /// (ADR 0038's hand-off means the surface only ever receives the lanes or the
    /// blit), so a `Bgra8` composite would warm a pipeline no frame can reach.
    ///
    /// **Why the compositor's two are here** (ADR 0040): a first frame with a group
    /// compiles [`Kind::Composite`] and [`Kind::Blit`] inside itself, and
    /// `Timings::phases` prices the pair at **0.75 ms at the minimum of 40 runs and
    /// 2.6 ms on the quietest one** on RADV — a third to a half of the 5 to 6 ms such a
    /// frame costs over its successors. Neither compile depends on the target's size,
    /// which is why no size hint could ever have moved them, and why the thread nobody
    /// blocks on is where they belong. A device that never composites pays for two
    /// pipelines it does not use, off the critical path, and reaches `is_warm` about
    /// 1.1 ms later.
    ///
    /// **[`Kind::Present`] is in the second set and only there** (ADR 0056). A
    /// presenter draws onto the surface and onto nothing else, so the pass has exactly
    /// one format it can ever be asked for and `WARM_FORMAT` is not it unless the
    /// surface negotiated that — which is why it is compiled whenever a present format
    /// exists, including when that format equals `WARM_FORMAT`, while the other three
    /// of the second set are skipped in that case as already compiled. A host that
    /// detaches its presenter therefore finds the pipeline built by this thread, and
    /// `Device::detach_presenter` compiles nothing itself; a host on a surface that
    /// never presents pays one pipeline on the thread nobody blocks on.
    ///
    /// **A refusal is kept rather than retried** (ADR 0042), and the guard records one
    /// on every exit path including an unwind — a warm-up that ends without saying so
    /// leaves every waiter on a `Condvar` nobody will notify, which is how an invalid
    /// shader used to hang the process instead of failing it.
    pub(super) fn warm_up_now(&self, present_format: Option<wgpu::TextureFormat>) {
        let guard = WarmUpGuard::new(self);
        let started = Instant::now();
        let compiled = self
            .get(Kind::RectOver, WARM_FORMAT)
            .and_then(|_| self.get(Kind::CoverOver, WARM_FORMAT))
            .and_then(|_| self.get(Kind::Composite, WARM_FORMAT))
            .and_then(|_| self.get(Kind::Blit, WARM_FORMAT))
            .and_then(|_| self.warm_presenting_lanes(present_format));
        guard.finish(match compiled {
            Ok(()) => WarmUp::Warm(started.elapsed()),
            Err(reason) => WarmUp::Refused(reason),
        });
    }

    /// The second set: what a device built for a surface compiles in the surface's own
    /// negotiated format. Nothing at all for a headless device.
    fn warm_presenting_lanes(
        &self,
        present_format: Option<wgpu::TextureFormat>,
    ) -> Result<(), PipelineProblem> {
        let Some(format) = present_format else {
            return Ok(());
        };
        if format != WARM_FORMAT {
            self.get(Kind::RectOver, format)?;
            self.get(Kind::CoverOver, format)?;
            self.get(Kind::Blit, format)?;
        }
        self.get(Kind::Present, format)?;
        Ok(())
    }

    /// Where the background warm-up has got to.
    pub(crate) fn warm_up(&self) -> WarmUp {
        self.lock().warm_up.clone()
    }

    /// How long the warm set took to compile, once it has.
    pub(crate) fn warm_duration(&self) -> Option<Duration> {
        let state = self.lock();
        match state.warm_up {
            WarmUp::Warm(duration) => Some(duration),
            _ => None,
        }
    }

    /// Block until the warm-up reports what became of it, and say what that was.
    /// Measurement support: startup numbers need a defined "fully warm" moment to be
    /// comparable.
    ///
    /// Returns for every outcome, including a warm-up that panicked
    /// ([`WarmUp::Abandoned`]) — see [`WarmUpGuard`].
    pub(crate) fn wait_until_warm(&self) -> WarmUp {
        let mut state = self.lock();
        while matches!(state.warm_up, WarmUp::Running) {
            state = self
                .warmed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.warm_up.clone()
    }
}

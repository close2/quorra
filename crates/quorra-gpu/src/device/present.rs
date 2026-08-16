//! The surface leaving the device and coming back (ADR 0056).
//!
//! Two methods and no third state to hold: the surface is either here, or it is out
//! with exactly one [`Presenter`], and `surface.rs`'s `SurfaceSlot` is where that is
//! written down. What this file owns is the handshake — what a presenter is given, what
//! it must prove to come back, and why neither call compiles anything.
//!
//! **Neither of these is on any critical path.** [`Device::detach_presenter`] clones
//! three `wgpu` handles and a reference count and moves the surface state; it asks the
//! pipeline store nothing,
//! so it cannot compile, cannot wait for the warm-up and cannot block. The one pipeline
//! a presenter draws with is in the warm set of every device built for a surface
//! (ADR 0043's second set, extended by ADR 0056), so it is built by the thread nobody
//! waits on — and if it is not yet, the first present builds it inline and says so in
//! [`PresentCost::compiled`], which is exactly what a first frame does with any lane.

use std::sync::Arc;

use super::Device;
use crate::present::{ForeignPresenter, Presenter};

impl Device {
    /// Take the surface out of this device, so that a window can be presented to from
    /// another thread while [`Device::render`] is running on this one.
    ///
    /// `None` for a headless device, and `None` on the second call — in both there is
    /// nothing to hand over. While the presenter is out, this device refuses
    /// [`Target::Surface`](crate::target::Target::Surface) and
    /// [`Device::invalidate_surface`] with
    /// [`RenderError::PresenterDetached`](crate::error::RenderError::PresenterDetached),
    /// by name: a device that drew nowhere and said nothing is principle 6's third
    /// state. Everything else the device does — `Target::Readback`,
    /// `Target::Texture`, uploads, the atlas — is untouched, and rendering into a
    /// host-owned texture that the presenter then puts on the window is the whole
    /// arrangement this exists for.
    ///
    /// **What it costs: four handle clones and a move.** No pipeline is compiled, no
    /// warm-up is waited for, nothing is acquired and nothing blocks.
    ///
    /// The presenter borrows nothing from this device — it holds its own clones of the
    /// `wgpu` device and queue and a shared reference to the pipeline store — so it may
    /// outlive it. Dropping the device first is legitimate and costs the presenter
    /// nothing but the ability to be attached back.
    #[must_use]
    pub fn detach_presenter(&mut self) -> Option<Presenter> {
        let surface = self.surface.detach()?;
        Some(Presenter::new(
            self.id,
            self.gpu.clone(),
            self.queue.clone(),
            Arc::clone(&self.pipelines),
            self.linear_sampler.clone(),
            surface,
        ))
    }

    /// Give the surface back. [`Target::Surface`](crate::target::Target::Surface) works
    /// again from the next frame.
    ///
    /// # Errors
    ///
    /// [`ForeignPresenter`] when the presenter came from another device, or when this
    /// device is not waiting for one (a headless device, or one that never detached).
    /// A surface's format was negotiated against the adapter *its* device chose, so
    /// attaching it elsewhere would put a swapchain nobody negotiated under a window —
    /// the same rule that stops a retained encode being replayed through another device
    /// (ADR 0048), and checked against the same device number.
    ///
    /// **The presenter comes back inside the error** and is recovered with
    /// [`ForeignPresenter::into_presenter`]: consuming it on the failing path would
    /// destroy the window's surface over a caller's mix-up.
    pub fn attach_presenter(&mut self, presenter: Presenter) -> Result<(), ForeignPresenter> {
        // Both questions before the presenter is consumed, so that the refusal can hand
        // it back whole. The second cannot fail on its own for a presenter carrying this
        // device's number — numbers are never reused and only one presenter is ever out
        // — but it is asked rather than assumed, because "cannot" here is a proof about
        // this crate's code and not a guarantee the type system makes.
        if presenter.device_id() != self.id || !self.surface.is_detached() {
            return Err(ForeignPresenter::new(presenter, self.id));
        }
        self.surface.attach(presenter.into_surface());
        Ok(())
    }
}

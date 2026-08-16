//! What the swapchain answered instead of handing over a texture.
//!
//! The vocabulary of
//! [`RenderError::SurfaceUnavailable`](crate::error::RenderError::SurfaceUnavailable),
//! and the one enum here whose completeness is not ours to argue: its five variants are
//! exactly the five non-success arms of `wgpu` 30's `CurrentSurfaceTexture`, mapped one
//! for one in `surface.rs`. The two arms that *do* carry a texture — `Success` and
//! `Suboptimal` — are frames, not refusals; a `Suboptimal` one is drawn and the surface
//! is reconfigured before the next.

/// Why the surface could not provide a texture for this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceProblem {
    /// Acquiring the next frame timed out; the next render reconfigures the surface,
    /// so trying again is the fix. (Reconfigured rather than merely retried because a
    /// timeout can mean a swapchain wedged behind unsignalled acquire semaphores — a
    /// state a retry alone never leaves.)
    Timeout,
    /// The surface no longer matches the window; the next render reconfigures it, so
    /// trying again is the fix.
    Outdated,
    /// The surface is gone and must be recreated with
    /// [`Device::for_surface`](crate::device::Device::for_surface).
    Lost,
    /// The window is occluded; there is nothing to present to right now.
    Occluded,
    /// `wgpu`'s validation refused the acquire itself.
    ///
    /// It carries no detail because `wgpu` 30's arm carries none: the message went to an
    /// error scope or to the uncaptured-error handler at the moment it was raised, and
    /// what reaches this call site is the bare fact that one was. That is a gap in what
    /// this refusal can name, and it is named here rather than papered over.
    Validation,
}

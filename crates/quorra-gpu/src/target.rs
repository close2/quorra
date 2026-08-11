//! Three kinds of target, because there are three kinds of host.
//!
//! # Why three, and why this is the first milestone's work
//!
//! §6.1 of the brief measured an offscreen frame and found that **between 55% and 92%
//! of it is paid before any of the page is drawn** — 3.48 ms of 6.34 at 1×, 26.73 of
//! 29.13 at 4×. Most of that scales with *bytes*: 26.7 ms for a 32 MB target is about
//! 1.2 GB/s, which is what a mapped readback and a demultiply cost.
//!
//! **A window presenting to a swapchain does not pay the readback at all.** That is why
//! the brief asks for three target kinds rather than one, and why the ranking it gives
//! puts the surface and texture paths first: they delete the largest single item in the
//! frame.
//!
//! # The rule that holds for all three
//!
//! **Render onto transparency, always.** ISO 32000-2 §11.4.7 makes the page group
//! *isolated*, and a finished page is composited onto the medium by the caller after we
//! return it — painting the medium first is a different picture. Every target is
//! cleared to transparent before the scene draws, including a `Texture` the host owns.
//! A non-isolated group inside the page (§11.4.4, ADR 0019) seeds its own buffer from
//! its backdrop, never from the target: the page's own initial backdrop stays
//! transparent, which is what §11.4.7 requires of it.

/// Where a frame's pixels go.
#[derive(Debug)]
pub enum Target<'a> {
    /// Tier 1: the caller wants the pixels. The frame carries a
    /// [`Raster`](crate::frame::Raster) — straight-alpha RGBA8, converted once at this
    /// boundary (§3 of the brief).
    ///
    /// This is the correctness oracle's path and the expensive one: it pays the
    /// copy-out, the map, and the demultiply, and [`Timings::readback`] prices exactly
    /// that. §6.1's numbers say this cost dominates an offscreen frame, which is why
    /// the other two tiers exist.
    ///
    /// [`Timings::readback`]: crate::frame::Timings::readback
    Readback,
    /// Tier 2: draw into the surface the device was constructed with
    /// ([`Device::for_surface`]) and present. No readback.
    ///
    /// [`Device::for_surface`]: crate::device::Device::for_surface
    Surface,
    /// Tier 3: a texture the host owns and composites itself. No readback.
    ///
    /// The texture must be 2D, single-sampled, `Rgba8Unorm`, sized exactly to the
    /// viewport, and created with `RENDER_ATTACHMENT` usage on this same device; a
    /// texture that is not is refused with an error naming which requirement failed,
    /// before anything is drawn. Its previous contents are replaced: the device clears
    /// it to transparent, because a page renders onto transparency, always.
    Texture(&'a wgpu::Texture),
}

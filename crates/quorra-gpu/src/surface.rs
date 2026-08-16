//! The surface's lifecycle: format negotiation at construction, configuration and
//! acquisition per frame — and **who holds it**, which since ADR 0056 is a question
//! with three answers rather than two.
//!
//! Tier 2 of §2.4 — the target kind that pays no readback at all, which §6.1 measured
//! as the largest single item in an offscreen frame. Everything here serves one rule:
//! a surface problem is a typed refusal ([`RenderError::SurfaceUnavailable`]) the
//! caller can retry on, never a quietly skipped frame.

use crate::error::{DeviceError, RenderError, SurfaceProblem};

/// Where the surface is: never existed, held by the device, or out with a
/// [`Presenter`](crate::present::Presenter).
///
/// One field rather than an `Option` beside a flag, because the third case is a state
/// and not a modifier: `Target::Surface` is refused in two of the three and **in
/// different words**, which is principle 6 applied to a distinction a host can act on.
/// A host that detached its presenter and forgot is not a host that built a headless
/// device, and telling it the wrong one costs it the diagnosis.
pub(crate) enum SurfaceSlot {
    /// [`Device::headless`](crate::device::Device::headless): no surface, and there
    /// never was one.
    Headless,
    /// The device holds it, which is what lets
    /// [`Target::Surface`](crate::target::Target::Surface) draw.
    Held(SurfaceState),
    /// A [`Presenter`](crate::present::Presenter) holds it, on whatever thread the host
    /// has put it on.
    Detached,
}

impl std::fmt::Debug for SurfaceSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Headless => f.write_str("Headless"),
            Self::Held(state) => f.debug_tuple("Held").field(state).finish(),
            Self::Detached => f.write_str("Detached"),
        }
    }
}

impl SurfaceSlot {
    /// The surface a frame is about to draw into, or the refusal naming why there is
    /// none: [`RenderError::NoSurface`] for a headless device,
    /// [`RenderError::PresenterDetached`] for one whose presenter is out.
    pub(crate) fn held_mut(&mut self) -> Result<&mut SurfaceState, RenderError> {
        match self {
            Self::Held(state) => Ok(state),
            Self::Headless => Err(RenderError::NoSurface),
            Self::Detached => Err(RenderError::PresenterDetached),
        }
    }

    /// The format the warm-up thread compiles the presenting lanes for (ADR 0043), or
    /// `None` when there is no surface to learn one from. Asked at construction, where
    /// the slot cannot yet be [`SurfaceSlot::Detached`].
    pub(crate) fn format(&self) -> Option<wgpu::TextureFormat> {
        match self {
            Self::Held(state) => Some(state.format()),
            Self::Headless | Self::Detached => None,
        }
    }

    /// Take the surface out, leaving [`SurfaceSlot::Detached`] behind. `None` for a
    /// headless device and `None` on the second call — in both there is nothing to
    /// hand over, and neither is changed by being asked.
    pub(crate) fn detach(&mut self) -> Option<SurfaceState> {
        match std::mem::replace(self, Self::Detached) {
            Self::Held(state) => Some(state),
            other => {
                *self = other;
                None
            }
        }
    }

    /// Whether this slot is waiting for a surface to come back — the question
    /// [`Device::attach_presenter`](crate::device::Device::attach_presenter) asks before
    /// it consumes the presenter, so that a refusal can hand it back intact.
    pub(crate) fn is_detached(&self) -> bool {
        matches!(self, Self::Detached)
    }

    /// Put a surface back. Only ever called after [`SurfaceSlot::is_detached`] said yes;
    /// the assertion states that rather than leaving it to be inferred.
    pub(crate) fn attach(&mut self, state: SurfaceState) {
        debug_assert!(
            self.is_detached(),
            "a surface may only be attached to a device that handed one out"
        );
        *self = Self::Held(state);
    }
}

/// The surface and the choices made for it at construction.
pub(crate) struct SurfaceState {
    surface: wgpu::Surface<'static>,
    format: wgpu::TextureFormat,
    present_mode: wgpu::PresentMode,
    alpha_mode: wgpu::CompositeAlphaMode,
    /// The size the surface is currently configured for, if any.
    configured: Option<(u32, u32)>,
    /// Set when the surface reported itself suboptimal, outdated or timed out, or
    /// when [`SurfaceState::invalidate`] was called; forces a reconfigure — a fresh
    /// swapchain — on the next frame.
    needs_reconfigure: bool,
    /// Whether the last [`SurfaceState::acquire`] configured before it acquired. A fact
    /// about the call that just happened, kept because a swapchain replaced under a
    /// presenter is the one thing that makes one present cost unlike its neighbours,
    /// and [`PresentCost`](crate::present::PresentCost) reports it as the exact thing it
    /// is rather than as a duration it would be inferred from.
    reconfigured: bool,
}

impl std::fmt::Debug for SurfaceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurfaceState")
            .field("format", &self.format)
            .field("configured", &self.configured)
            .finish_non_exhaustive()
    }
}

impl SurfaceState {
    /// Negotiate the surface's format and modes against the chosen adapter.
    ///
    /// # Errors
    ///
    /// [`DeviceError::SurfaceUnsupported`] when the adapter offers no format at all
    /// for this surface.
    pub(crate) fn new(
        surface: wgpu::Surface<'static>,
        adapter: &wgpu::Adapter,
        adapter_name: &str,
    ) -> Result<Self, DeviceError> {
        let caps = surface.get_capabilities(adapter);
        let Some(&format) = caps
            .formats
            .iter()
            .find(|f| **f == wgpu::TextureFormat::Bgra8Unorm)
            .or_else(|| caps.formats.first())
        else {
            return Err(DeviceError::SurfaceUnsupported {
                adapter: adapter_name.to_owned(),
            });
        };
        // Fifo is the one mode WebGPU guarantees; the alpha mode preference order is
        // the capability list's own.
        let alpha_mode = caps
            .alpha_modes
            .first()
            .copied()
            .unwrap_or(wgpu::CompositeAlphaMode::Auto);
        Ok(Self {
            surface,
            format,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            configured: None,
            needs_reconfigure: false,
            reconfigured: false,
        })
    }

    /// The size this surface is configured for, if it has been configured at all.
    ///
    /// What a detached [`Presenter`](crate::present::Presenter) starts out believing the
    /// window is: `None` on a device that has not presented a frame yet, and then the
    /// presenter has nothing to present at until the host says
    /// ([`Presenter::resize`](crate::present::Presenter::resize)). Guessing a size here
    /// would configure a swapchain for a window nobody described.
    pub(crate) fn configured_size(&self) -> Option<(u32, u32)> {
        self.configured
    }

    /// Whether the last [`SurfaceState::acquire`] replaced the swapchain first.
    pub(crate) fn reconfigured(&self) -> bool {
        self.reconfigured
    }

    /// The format negotiated at construction — what every frame presented here is
    /// rendered in, and therefore what the warm-up thread compiles the presenting
    /// lanes for (ADR 0043).
    pub(crate) fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Force a reconfigure before the next acquire, replacing the swapchain.
    ///
    /// Called when the presentation stack may hold state a retry cannot clear: a
    /// frame that failed after its texture was acquired (the drop leaves an acquire
    /// semaphore no submission will wait on), or a host asking through
    /// [`Device::invalidate_surface`](crate::device::Device::invalidate_surface).
    pub(crate) fn invalidate(&mut self) {
        self.needs_reconfigure = true;
    }

    /// Configure (when the size changed or the surface asked for it) and acquire the
    /// next texture.
    pub(crate) fn acquire(
        &mut self,
        gpu: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Result<wgpu::SurfaceTexture, RenderError> {
        self.reconfigured = self.configured != Some((width, height)) || self.needs_reconfigure;
        if self.reconfigured {
            self.surface.configure(
                gpu,
                &wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format: self.format,
                    width,
                    height,
                    present_mode: self.present_mode,
                    desired_maximum_frame_latency: 2,
                    alpha_mode: self.alpha_mode,
                    view_formats: Vec::new(),
                    color_space: wgpu::SurfaceColorSpace::Auto,
                },
            );
            self.configured = Some((width, height));
            self.needs_reconfigure = false;
        }
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => Ok(texture),
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                // Still a correct frame; reconfigure next time for the better one.
                self.needs_reconfigure = true;
                Ok(texture)
            }
            wgpu::CurrentSurfaceTexture::Timeout => {
                // A timeout can mean a swapchain whose images are all stuck behind
                // acquire semaphores nothing will ever signal — a state a retry
                // alone never leaves (the viewer observed exactly this: every
                // subsequent acquire timing out, permanently). Reconfiguring
                // replaces the swapchain, which is the one recovery wgpu offers.
                self.needs_reconfigure = true;
                Err(RenderError::SurfaceUnavailable {
                    reason: SurfaceProblem::Timeout,
                })
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.needs_reconfigure = true;
                Err(RenderError::SurfaceUnavailable {
                    reason: SurfaceProblem::Outdated,
                })
            }
            wgpu::CurrentSurfaceTexture::Occluded => Err(RenderError::SurfaceUnavailable {
                reason: SurfaceProblem::Occluded,
            }),
            wgpu::CurrentSurfaceTexture::Lost => Err(RenderError::SurfaceUnavailable {
                reason: SurfaceProblem::Lost,
            }),
            wgpu::CurrentSurfaceTexture::Validation => Err(RenderError::SurfaceUnavailable {
                reason: SurfaceProblem::Validation,
            }),
        }
    }
}

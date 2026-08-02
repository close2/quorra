//! The surface's lifecycle: format negotiation at construction, configuration and
//! acquisition per frame.
//!
//! Tier 2 of §2.4 — the target kind that pays no readback at all, which §6.1 measured
//! as the largest single item in an offscreen frame. Everything here serves one rule:
//! a surface problem is a typed refusal ([`RenderError::SurfaceUnavailable`]) the
//! caller can retry on, never a quietly skipped frame.

use crate::error::{DeviceError, RenderError, SurfaceProblem};

/// The surface and the choices made for it at construction.
pub(crate) struct SurfaceState {
    surface: wgpu::Surface<'static>,
    format: wgpu::TextureFormat,
    present_mode: wgpu::PresentMode,
    alpha_mode: wgpu::CompositeAlphaMode,
    /// The size the surface is currently configured for, if any.
    configured: Option<(u32, u32)>,
    /// Set when the surface reported itself suboptimal or outdated; forces a
    /// reconfigure on the next frame.
    needs_reconfigure: bool,
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
        })
    }

    /// Configure (when the size changed or the surface asked for it) and acquire the
    /// next texture.
    pub(crate) fn acquire(
        &mut self,
        gpu: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Result<wgpu::SurfaceTexture, RenderError> {
        if self.configured != Some((width, height)) || self.needs_reconfigure {
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
            wgpu::CurrentSurfaceTexture::Timeout => Err(RenderError::SurfaceUnavailable {
                reason: SurfaceProblem::Timeout,
            }),
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

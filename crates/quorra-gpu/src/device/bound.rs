//! What a frame draws into: the three targets behind one [`Bound`], the contract each
//! must satisfy before a pass is recorded, and what happens to one after a failure.
//!
//! The three differ in who owns the texture — this frame, the caller, or the swapchain
//! — and that ownership is the whole of what the rest of a frame needs to know about
//! them. It is also why **an acquired surface texture is the last thing a frame takes
//! and the first thing a failure has to answer for**: a swapchain texture acquired and
//! then dropped unpresented leaves an acquire semaphore no submission will ever wait
//! on, and enough of those time out every later acquire, permanently. The caller
//! measured that. So every refusal a scene can earn is taken before `bind_target` is
//! called, and `abandon_frame` invalidates the surface for the one class of failure
//! that can still happen afterwards.
//!
//! `surface.rs` owns the swapchain's own lifecycle — configuration, its reported
//! problems, and the reconfiguration this module asks it for.

use super::Device;
use crate::error::RenderError;
use crate::pipeline::WARM_FORMAT;
use crate::target::Target;
use crate::viewport::Viewport;

/// What a render pass draws into for one frame.
pub(super) enum Bound<'a> {
    /// A texture this frame created (`Target::Readback`).
    Owned(wgpu::Texture),
    /// The caller's texture (`Target::Texture`).
    Borrowed(&'a wgpu::Texture),
    /// The acquired swapchain texture (`Target::Surface`).
    Acquired(wgpu::SurfaceTexture),
}

impl Bound<'_> {
    pub(super) fn texture(&self) -> &wgpu::Texture {
        match self {
            Bound::Owned(t) => t,
            Bound::Borrowed(t) => t,
            Bound::Acquired(s) => &s.texture,
        }
    }
}

impl Device {
    /// Force the surface to be reconfigured — a fresh swapchain — before the next
    /// [`Target::Surface`] frame.
    ///
    /// The host's lever for a presentation stack it suspects is wedged: the surface
    /// itself reports [`SurfaceProblem`](crate::error::SurfaceProblem)s and asks for
    /// its own reconfiguration where it can tell, but a host that knows better —
    /// after a run of refusals, or a compositor event this library cannot see — need
    /// not wait for that or fake a resize. Costs nothing until the next surface
    /// frame, which pays one reconfigure.
    ///
    /// # Errors
    ///
    /// [`RenderError::NoSurface`] on a device constructed with
    /// [`Device::headless`] — asking to invalidate a surface that cannot exist is a
    /// caller bug, and hiding it would hide the defect.
    pub fn invalidate_surface(&mut self) -> Result<(), RenderError> {
        let Some(surface) = self.surface.as_mut() else {
            return Err(RenderError::NoSurface);
        };
        surface.invalidate();
        Ok(())
    }

    /// Give up a bound target after a failure, and pass the error through.
    ///
    /// Dropping an acquired-but-unpresented swapchain texture leaves the swapchain
    /// an acquire semaphore no submission will ever wait on, and enough of those
    /// exhaust it — every later acquire times out. Invalidating the surface here
    /// bounds the damage of a post-acquire failure at one lost frame: the next
    /// frame reconfigures, which replaces the swapchain.
    pub(super) fn abandon_frame(&mut self, bound: Bound<'_>, error: RenderError) -> RenderError {
        if matches!(bound, Bound::Acquired(_)) {
            drop(bound);
            if let Some(surface) = self.surface.as_mut() {
                surface.invalidate();
            }
        }
        error
    }

    /// Bind the frame's target, validating a caller texture against its contract.
    pub(super) fn bind_target<'a>(
        &mut self,
        into: &Target<'a>,
        viewport: &Viewport<'_>,
    ) -> Result<Bound<'a>, RenderError> {
        match into {
            Target::Readback => Ok(Bound::Owned(self.gpu.create_texture(
                &wgpu::TextureDescriptor {
                    label: Some("quorra readback target"),
                    size: wgpu::Extent3d {
                        width: viewport.width,
                        height: viewport.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: WARM_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                },
            ))),
            Target::Texture(texture) => {
                Self::validate_texture(texture, viewport)?;
                Ok(Bound::Borrowed(texture))
            }
            Target::Surface => {
                let Some(state) = self.surface.as_mut() else {
                    return Err(RenderError::NoSurface);
                };
                Ok(Bound::Acquired(state.acquire(
                    &self.gpu,
                    viewport.width,
                    viewport.height,
                )?))
            }
        }
    }

    /// The `Target::Texture` contract, checked before anything draws.
    fn validate_texture(
        texture: &wgpu::Texture,
        viewport: &Viewport<'_>,
    ) -> Result<(), RenderError> {
        if texture.format() != WARM_FORMAT {
            return Err(RenderError::TextureFormat {
                got: texture.format(),
            });
        }
        if texture.width() != viewport.width || texture.height() != viewport.height {
            return Err(RenderError::TextureSize {
                got_width: texture.width(),
                got_height: texture.height(),
                need_width: viewport.width,
                need_height: viewport.height,
            });
        }
        if !texture
            .usage()
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            return Err(RenderError::TextureUsage);
        }
        if texture.dimension() != wgpu::TextureDimension::D2
            || texture.sample_count() != 1
            || texture.depth_or_array_layers() != 1
        {
            return Err(RenderError::TextureShape);
        }
        Ok(())
    }
}

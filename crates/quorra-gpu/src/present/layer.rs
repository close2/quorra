//! What a host hands a [`Presenter`](super::Presenter) to put on the window, and the
//! contract every one of them is held to before anything is acquired.
//!
//! A window is a page **and** chrome — a sidebar, a selection, a caret — so a present
//! takes a slice rather than one texture, and the layers land in the order they are
//! given, each under its own placement and filter. What that buys the caller is that
//! the chrome stays exactly as stale as the page and no worse while the page is
//! re-placed under a new affine without being redrawn.

use quorra_scene::{Affine, ImageFilter};

use crate::error::{LayerProblem, RenderError};
use crate::pipeline::WARM_FORMAT;

/// One finished raster to put on the window, and where it goes.
///
/// The texture must be one **this device** rendered into through
/// [`Target::Texture`](crate::target::Target::Texture): `Rgba8Unorm`, single-sampled,
/// 2D, one array layer, non-empty — and carrying `TEXTURE_BINDING` as well as
/// `RENDER_ATTACHMENT`, because rendering into a texture and sampling out of one are
/// two different usages and only the first is what a render target needs. Anything else
/// is refused by name ([`LayerProblem`]) rather than presented wrongly.
///
/// A layer's pixels are **premultiplied**, which is what a `Target::Texture` frame
/// leaves behind — §3's straight-alpha conversion happens at readback and nowhere else.
#[derive(Debug, Clone, Copy)]
pub struct Layer<'a> {
    /// The raster.
    pub texture: &'a wgpu::Texture,
    /// Where it goes: the transform from the texture's own texel space to the surface's
    /// pixels. [`Affine::IDENTITY`] puts texel (0, 0) at the window's top-left corner
    /// and one texel on one pixel.
    ///
    /// This is the reprojection seam. A host whose render is late presents the last
    /// finished page under the affine that carries it from where it was rendered to
    /// where the view now is, and when the next render lands the affine is the identity
    /// again.
    pub placement: Affine,
    /// How the texture is sampled where the placement does not land on texel centres.
    ///
    /// [`ImageFilter::Nearest`] is a texel fetch — exact, and identical on every
    /// adapter; [`ImageFilter::Linear`] is the hardware sampler, whose interpolation
    /// precision is the driver's. The same two the image lane offers, decided by the
    /// same caller (§4.5).
    pub filter: ImageFilter,
}

impl Layer<'_> {
    /// The layer's extent in texels, as the shader's floats.
    #[allow(clippy::cast_precision_loss)] // texture extents are exact in f32
    pub(super) fn extent(&self) -> [f32; 2] {
        [self.texture.width() as f32, self.texture.height() as f32]
    }

    /// The contract, checked before anything is acquired — the same order
    /// `Device::render` takes its refusals in, and for the same reason: a swapchain
    /// texture acquired and then dropped unpresented leaves an acquire semaphore no
    /// submission will wait on, and enough of those time out every later acquire.
    ///
    /// The one question not asked here is which device made the texture, because
    /// `wgpu::Texture` does not answer it; `super::pass` asks `wgpu` instead, still
    /// before the acquire.
    ///
    /// # Errors
    ///
    /// [`RenderError::LayerRefused`] naming this layer's index and which clause of the
    /// contract it broke.
    pub(super) fn check(&self, index: usize) -> Result<Affine, RenderError> {
        self.inverse()
            .map_err(|reason| RenderError::LayerRefused { index, reason })
    }

    /// The inverse of the placement — what the fragment stage maps a surface pixel back
    /// through — or the first thing about this layer that did not hold.
    fn inverse(&self) -> Result<Affine, LayerProblem> {
        let texture = self.texture;
        if texture.format() != WARM_FORMAT {
            return Err(LayerProblem::Format {
                got: texture.format(),
            });
        }
        if !texture
            .usage()
            .contains(wgpu::TextureUsages::TEXTURE_BINDING)
        {
            return Err(LayerProblem::NotSampleable);
        }
        if texture.dimension() != wgpu::TextureDimension::D2
            || texture.sample_count() != 1
            || texture.depth_or_array_layers() != 1
        {
            return Err(LayerProblem::Shape);
        }
        // Nothing asks whether the texture has pixels: `wgpu` refuses to create one
        // without any (WebGPU requires every extent to be at least 1), so the shader's
        // division by the extent under a linear filter cannot meet a zero.
        //
        // No identity fallback, deliberately (§4.7, and `Affine::invert`'s own note): a
        // degenerate placement substituted by the identity is the plausible-looking
        // wrong window, which is worse than a refusal a host can act on.
        self.placement.invert().ok_or(LayerProblem::Placement)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)] // test-file policy: a fixture that cannot run must fail loudly
mod tests {
    use quorra_scene::{Affine, ImageFilter};

    use super::Layer;
    use crate::device::Device;
    use crate::error::{LayerProblem, RenderError};
    use crate::pipeline::WARM_FORMAT;
    use crate::startup::Options;

    /// The software adapter, as everywhere in this crate's unit tests.
    fn device() -> Device {
        Device::headless(&Options {
            adapter: Some("llvmpipe".into()),
            ..Options::default()
        })
        .expect("llvmpipe is present wherever this suite runs")
    }

    /// A texture with everything the contract asks for, then one field changed by the
    /// caller of this helper.
    fn texture(gpu: &wgpu::Device, descriptor: &wgpu::TextureDescriptor<'_>) -> wgpu::Texture {
        gpu.create_texture(descriptor)
    }

    /// What a host that read the documentation creates: a target it can render into and
    /// then present.
    fn presentable(width: u32, height: u32) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("a layer"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: WARM_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        }
    }

    /// Which problem a layer earns, or `None` when it is accepted.
    fn refusal(texture: &wgpu::Texture, placement: Affine) -> Option<LayerProblem> {
        let layer = Layer {
            texture,
            placement,
            filter: ImageFilter::Nearest,
        };
        match layer.check(3) {
            Ok(_) => None,
            Err(RenderError::LayerRefused { index, reason }) => {
                assert_eq!(index, 3, "the refusal names the layer it was asked about");
                Some(reason)
            }
            Err(other) => panic!("a layer refusal is a LayerRefused, got {other}"),
        }
    }

    /// The accepting half, without which every assertion below could pass by refusing
    /// everything (ADR 0052: verify a gate in both directions).
    #[test]
    fn a_texture_rendered_for_presenting_is_accepted() {
        let device = device();
        let (gpu, _) = device.wgpu();
        let texture = texture(gpu, &presentable(64, 32));
        assert!(refusal(&texture, Affine::translate(4.0, 9.0)).is_none());
    }

    /// The trap this contract exists to name: `Target::Texture` needs
    /// `RENDER_ATTACHMENT` and sampling needs `TEXTURE_BINDING`, so a texture that has
    /// been a render target every frame can still be unpresentable.
    #[test]
    fn a_target_that_cannot_be_sampled_is_refused_by_name() {
        let device = device();
        let (gpu, _) = device.wgpu();
        let texture = texture(
            gpu,
            &wgpu::TextureDescriptor {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                ..presentable(64, 32)
            },
        );
        assert_eq!(
            refusal(&texture, Affine::IDENTITY),
            Some(LayerProblem::NotSampleable)
        );
    }

    /// The format is the one `Target::Texture` renders in, and a layer in another one
    /// says which it was.
    #[test]
    fn a_layer_in_another_format_names_the_format_it_has() {
        let device = device();
        let (gpu, _) = device.wgpu();
        let texture = texture(
            gpu,
            &wgpu::TextureDescriptor {
                format: wgpu::TextureFormat::Bgra8Unorm,
                ..presentable(64, 32)
            },
        );
        assert_eq!(
            refusal(&texture, Affine::IDENTITY),
            Some(LayerProblem::Format {
                got: wgpu::TextureFormat::Bgra8Unorm
            })
        );
    }

    /// Multisampled, or an array, or 1D: the shape a `texture_2d<f32>` binding cannot be.
    #[test]
    fn a_layer_of_the_wrong_shape_is_refused() {
        let device = device();
        let (gpu, _) = device.wgpu();
        let multisampled = texture(
            gpu,
            &wgpu::TextureDescriptor {
                sample_count: 4,
                ..presentable(64, 32)
            },
        );
        assert_eq!(
            refusal(&multisampled, Affine::IDENTITY),
            Some(LayerProblem::Shape)
        );
        let layered = texture(
            gpu,
            &wgpu::TextureDescriptor {
                size: wgpu::Extent3d {
                    width: 64,
                    height: 32,
                    depth_or_array_layers: 2,
                },
                ..presentable(64, 32)
            },
        );
        assert_eq!(
            refusal(&layered, Affine::IDENTITY),
            Some(LayerProblem::Shape)
        );
    }

    /// A placement with no inverse has no arithmetic to do, and the refusal says so
    /// rather than the presenter substituting an identity (§4.7).
    #[test]
    fn a_degenerate_or_non_finite_placement_is_refused() {
        let device = device();
        let (gpu, _) = device.wgpu();
        let texture = texture(gpu, &presentable(64, 32));
        assert_eq!(
            refusal(&texture, Affine::scale(0.0, 1.0)),
            Some(LayerProblem::Placement)
        );
        assert_eq!(
            refusal(&texture, Affine::translate(f32::NAN, 0.0)),
            Some(LayerProblem::Placement)
        );
    }

    /// The order the questions are asked in is part of the contract: a texture that
    /// breaks two of them reports the first, so a host fixing them one at a time is not
    /// told about the second before the first is gone.
    #[test]
    fn the_first_broken_clause_is_the_one_reported() {
        let device = device();
        let (gpu, _) = device.wgpu();
        let texture = texture(
            gpu,
            &wgpu::TextureDescriptor {
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                ..presentable(64, 32)
            },
        );
        assert!(matches!(
            refusal(&texture, Affine::IDENTITY),
            Some(LayerProblem::Format { .. })
        ));
    }
}

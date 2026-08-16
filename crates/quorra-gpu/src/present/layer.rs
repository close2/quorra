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

/// A layer that passed its contract, and the two things the pass makes of its
/// placement: the inverse the fragment stage maps a pixel back through, and the
/// rectangle of the target the vertex stage draws (ADR 0058).
///
/// One type rather than two calls because the second is only meaningful after the
/// first: [`Layer::device_bounds`] assumes a finite, invertible placement, and here it
/// cannot be reached without one.
#[derive(Debug, Clone, Copy)]
pub(super) struct Placed {
    /// The placement's inverse: a surface pixel → the layer's texel space.
    pub(super) inverse: Affine,
    /// Where this layer can reach on the target, in device pixels: left, top, right,
    /// bottom. Whole pixels, clamped to the target, and never smaller than the set of
    /// pixels whose centres the fragment stage would accept.
    pub(super) bounds: [f32; 4],
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
    /// `target` is the size the present will acquire at — a number, not a texture:
    /// nothing is acquired here and a refusal still costs the swapchain nothing.
    ///
    /// # Errors
    ///
    /// [`RenderError::LayerRefused`] naming this layer's index and which clause of the
    /// contract it broke.
    pub(super) fn check(&self, index: usize, target: (u32, u32)) -> Result<Placed, RenderError> {
        let inverse = self
            .inverse()
            .map_err(|reason| RenderError::LayerRefused { index, reason })?;
        Ok(Placed {
            inverse,
            bounds: self.device_bounds(target),
        })
    }

    /// Where the placement puts this layer's own rectangle on the target, grown outward
    /// to whole pixels and clamped to the target's own extent.
    ///
    /// **A bound, and deliberately a loose one.** The fragment stage decides which texel
    /// a pixel gets and whether it gets one at all; this only says which pixels are
    /// worth asking about. So it must never be too small, and the two ways it is kept
    /// safe are the outward pixel each side — far larger than any rounding difference
    /// between a rasteriser's edge functions and the shader's inverse map — and the
    /// axis-aligned box, which contains the placement's parallelogram whatever the
    /// linear part does. A rotated placement therefore pays for its box rather than for
    /// its shape (ADR 0058 records what that costs).
    ///
    /// **In `f64`, so that there is no non-finite case to have an opinion about.**
    /// [`Layer::inverse`] has already refused a non-finite placement, and `wgpu` will
    /// not make a texture with an empty extent, so every corner here is a sum of three
    /// products of finite `f32`s — which cannot leave `f64`'s range. The clamp then
    /// puts the result inside the target, where `f32` holds every whole number exactly.
    #[allow(clippy::cast_possible_truncation)] // clamped to the target's own extent
    fn device_bounds(&self, (width, height): (u32, u32)) -> [f32; 4] {
        let place = self.placement;
        // §8.3.3's `[a b c d e f]`, applied in `f64`: the same arithmetic
        // `Affine::apply` does, widened.
        let corner = |across: f64, down: f64| {
            (
                f64::from(place.a) * across + f64::from(place.c) * down + f64::from(place.e),
                f64::from(place.b) * across + f64::from(place.d) * down + f64::from(place.f),
            )
        };
        let texels_wide = f64::from(self.texture.width());
        let texels_tall = f64::from(self.texture.height());
        let corners = [
            corner(0.0, 0.0),
            corner(texels_wide, 0.0),
            corner(0.0, texels_tall),
            corner(texels_wide, texels_tall),
        ];
        let span = |pick: fn(&(f64, f64)) -> f64, limit: u32| {
            let low = corners.iter().map(pick).fold(f64::INFINITY, f64::min);
            let high = corners.iter().map(pick).fold(f64::NEG_INFINITY, f64::max);
            let limit = f64::from(limit);
            (
                (low.floor() - 1.0).clamp(0.0, limit) as f32,
                (high.ceil() + 1.0).clamp(0.0, limit) as f32,
            )
        };
        let (left, right) = span(|c| c.0, width);
        let (top, bottom) = span(|c| c.1, height);
        [left, top, right, bottom]
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
// Test-file policy: a fixture that cannot run must fail loudly. And every bound below is
// a whole number of pixels — the arithmetic ends in `floor`, `ceil` and a clamp to an
// integral extent — so exact equality is the right assertion, where an epsilon would
// hide a defect of less than a pixel.
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::cast_precision_loss
)]
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

    /// A window for the checks below to place layers on.
    const TARGET: (u32, u32) = (640, 480);

    /// Which problem a layer earns, or `None` when it is accepted.
    fn refusal(texture: &wgpu::Texture, placement: Affine) -> Option<LayerProblem> {
        let layer = Layer {
            texture,
            placement,
            filter: ImageFilter::Nearest,
        };
        match layer.check(3, TARGET) {
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

    /// The rectangle a placement puts a layer in, for the checks below.
    fn bounds(width: u32, height: u32, placement: Affine) -> [f32; 4] {
        let device = device();
        let (gpu, _) = device.wgpu();
        let texture = texture(gpu, &presentable(width, height));
        Layer {
            texture: &texture,
            placement,
            filter: ImageFilter::Nearest,
        }
        .device_bounds(TARGET)
    }

    /// **The bound is the placement's own rectangle**, grown by the pixel each side that
    /// keeps the fragment stage the only thing deciding which pixel gets a texel.
    ///
    /// A 64×32 layer at (100, 50) reaches device x 100..164 and y 50..82; the drawn
    /// rectangle is one pixel wider each way, and every number in it is arithmetic on
    /// the placement rather than anything an adapter said.
    #[test]
    fn the_drawn_rectangle_is_where_the_placement_puts_the_layer() {
        assert_eq!(
            bounds(64, 32, Affine::translate(100.0, 50.0)),
            [99.0, 49.0, 165.0, 83.0]
        );
    }

    /// A scale is in it: 64 texels at 2× is 128 pixels, so a layer that would fit the
    /// window at the identity need not fit it scaled.
    #[test]
    fn a_scaled_placement_bounds_the_scaled_rectangle() {
        let placement = Affine::scale(2.0, 2.0).then(Affine::translate(10.0, 20.0));
        assert_eq!(bounds(64, 32, placement), [9.0, 19.0, 139.0, 85.0]);
    }

    /// **The window is the ceiling, and zero the floor.** A placement that puts most of
    /// a layer off the screen draws only the part that is on it, and a layer entirely
    /// off it draws an empty rectangle rather than a negative one.
    #[test]
    fn the_bound_is_clamped_to_the_target() {
        let mostly_off = bounds(64, 32, Affine::translate(-40.0, -20.0));
        assert_eq!(mostly_off, [0.0, 0.0, 25.0, 13.0]);
        let entirely_off = bounds(64, 32, Affine::translate(-400.0, -200.0));
        assert_eq!(entirely_off, [0.0, 0.0, 0.0, 0.0]);
        let past_the_far_corner = bounds(64, 32, Affine::translate(2_000.0, 2_000.0));
        assert_eq!(
            past_the_far_corner,
            [
                TARGET.0 as f32,
                TARGET.1 as f32,
                TARGET.0 as f32,
                TARGET.1 as f32
            ]
        );
    }

    /// A rotation has no axis-aligned rectangle of its own, so the bound is the box
    /// around the parallelogram — looser than the shape, and still never larger than the
    /// window (ADR 0058 prices that).
    ///
    /// A 64×32 layer turned a quarter turn about the origin and translated to (200, 100)
    /// occupies x 200−32..200 and y 100..100+64 — the extents exchanged.
    #[test]
    fn a_rotated_placement_is_bounded_by_its_box() {
        let quarter_turn = Affine {
            a: 0.0,
            b: 1.0,
            c: -1.0,
            d: 0.0,
            e: 200.0,
            f: 100.0,
        };
        assert_eq!(bounds(64, 32, quarter_turn), [167.0, 99.0, 201.0, 165.0]);
    }

    /// **A placement this contract accepts can still put a corner outside `f32`**, and
    /// the bound is where that is answered: `a = 1e38` has a finite determinant, so
    /// [`Layer::inverse`] admits it, and a 64-texel row under it reaches 6.4e39 — an
    /// infinity in `f32` and an ordinary number in `f64`. Computed in `f64` and clamped,
    /// what the shader gets is the window's own edge; computed in `f32` it would be a
    /// vertex no rasteriser can place, which is principle 6's silent nothing wearing an
    /// exponent. §4.7's rule is that data from a document is not trusted for being
    /// finite.
    #[test]
    fn a_corner_outside_f32_clamps_rather_than_overflowing() {
        let enormous = Affine {
            a: 1e38,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        };
        assert!(
            enormous.invert().is_some(),
            "the placement this test is about is one the layer contract accepts"
        );
        assert_eq!(bounds(64, 32, enormous), [0.0, 0.0, TARGET.0 as f32, 33.0]);
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

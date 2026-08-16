//! What one present records: a bind group per layer, a full-screen triangle each, one
//! submission — and the uniform bytes `present.wgsl` reads.
//!
//! Split from [`Presenter`](super::Presenter) along the seam that matters here: this
//! module is what a present *does to the device*, and its parent is who may ask. The
//! order the two halves run in is the parent's, and it is load-bearing — **everything
//! fallible happens before the swapchain texture is acquired**, because a texture
//! acquired and then dropped unpresented leaves an acquire semaphore no submission will
//! ever wait on, and enough of those time out every later acquire, permanently. So
//! [`bind_layers`] (which can refuse) takes no target, and [`record`] (which cannot)
//! takes one.

use quorra_scene::{Affine, ImageFilter};

use super::Layer;
use crate::error::{LayerProblem, RenderError};
use crate::pipeline::captured;

/// The 48 bytes `present.wgsl`'s `Params` reads, in its order: the placement's inverse
/// in §8.3.3's `[a b c d e f]` order across two `vec4f`s, the filter, and the layer's
/// extent in texels.
///
/// The `vec4f` split is the shader's arithmetic written down: `inv0` is the linear part
/// a fragment's centre is multiplied by, `inv1.xy` the translation added to it. The
/// third lane of `inv1` is the filter as a float because a `u32` there would make the
/// struct's uniform layout depend on a mixed-type rule for nothing gained.
fn params_bytes(inverse: Affine, filter: ImageFilter, extent: [f32; 2]) -> [u8; 48] {
    let filter = match filter {
        ImageFilter::Linear => 1.0,
        ImageFilter::Nearest => 0.0,
    };
    let values = [
        inverse.a, inverse.b, inverse.c, inverse.d, // inv0
        inverse.e, inverse.f, filter, 0.0, // inv1
        extent[0], extent[1], // source
    ];
    let mut bytes = [0_u8; 48];
    for (slot, value) in bytes.chunks_exact_mut(4).zip(values) {
        slot.copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// One layer's bind group: its uniform, its texture and the filtering sampler.
///
/// This is where a texture's **provenance** is checked, and it is checked by `wgpu`
/// rather than by us: `wgpu::Texture` (version 30) exposes size, format, usage and
/// shape and not the device that made it, so a texture from another device can only be
/// found by offering it to this one. The offer happens inside a validation error scope,
/// so what would otherwise be an uncaptured error — a panic, on whatever thread the
/// host put this presenter on — becomes [`LayerProblem::Unbindable`] instead (ADR 0042's
/// rule, applied to the one creation a presenter makes).
fn bind_layer(
    gpu: &wgpu::Device,
    queue: &wgpu::Queue,
    sampler: &wgpu::Sampler,
    layout: &wgpu::BindGroupLayout,
    layer: &Layer<'_>,
    inverse: Affine,
) -> Result<wgpu::BindGroup, LayerProblem> {
    let bytes = params_bytes(inverse, layer.filter, layer.extent());
    let uniform = gpu.create_buffer(&wgpu::BufferDescriptor {
        label: Some("quorra present placement"),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&uniform, 0, &bytes);
    let (bind, detail) = captured(gpu, || {
        // Mip 0 explicitly: a layer with more than one level is legitimate and the
        // presenter puts its full-size level on the window, rather than letting a
        // default view's level count decide what `textureLoad(…, 0)` even means.
        let view = layer.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("quorra present layer"),
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_mip_level: 0,
            mip_level_count: Some(1),
            base_array_layer: 0,
            array_layer_count: Some(1),
            ..Default::default()
        });
        gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra present"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    });
    match detail {
        Some(detail) => Err(LayerProblem::Unbindable { detail }),
        // The uniform buffer is not returned and is not dropped either: the bind group
        // holds it, which is what keeps it alive across the submission.
        None => Ok(bind),
    }
}

/// Every layer's bind group, in the order they will be drawn, or the first refusal.
///
/// Takes no target on purpose: nothing here has acquired anything, so a refusal costs
/// the swapchain nothing.
///
/// # Errors
///
/// [`RenderError::LayerRefused`] naming the layer's index and what did not hold.
pub(super) fn bind_layers(
    gpu: &wgpu::Device,
    queue: &wgpu::Queue,
    sampler: &wgpu::Sampler,
    layout: &wgpu::BindGroupLayout,
    layers: &[Layer<'_>],
) -> Result<Vec<wgpu::BindGroup>, RenderError> {
    let mut binds = Vec::with_capacity(layers.len());
    for (index, layer) in layers.iter().enumerate() {
        let inverse = layer.check(index)?;
        let bind = bind_layer(gpu, queue, sampler, layout, layer, inverse)
            .map_err(|reason| RenderError::LayerRefused { index, reason })?;
        binds.push(bind);
    }
    Ok(binds)
}

/// Clear the window and draw every layer over it, in order, and submit.
///
/// Infallible by construction, which is what makes it safe to run after the acquire:
/// the pipeline is already built, every bind group is already made and validated, and
/// what is left is recording and one submission.
///
/// The clear is not an optimisation to skip when a layer covers the window: a page is
/// rendered onto transparency (§3), the layers are put over that, and a window whose
/// previous contents survived where nothing was drawn would be showing a frame that is
/// partly two frames old. A present of no layers at all is a legitimate present — it
/// clears the window, which is what a host with nothing to show asked for.
pub(super) fn record(
    gpu: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &wgpu::RenderPipeline,
    target: &wgpu::TextureView,
    binds: &[wgpu::BindGroup],
) {
    let mut recorder = gpu.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("quorra present"),
    });
    {
        let mut pass = recorder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("quorra present"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        for bind in binds {
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }
    queue.submit(std::iter::once(recorder.finish()));
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)] // test-file policy: a fixture that cannot run must fail loudly
mod tests {
    use quorra_scene::{Affine, ImageFilter};

    use super::{Layer, bind_layers, params_bytes};
    use crate::device::Device;
    use crate::error::{LayerProblem, RenderError};
    use crate::pipeline::WARM_FORMAT;
    use crate::shaders;
    use crate::shaders::layout::{Lane, check};
    use crate::startup::Options;

    /// Two devices of **one** instance, on the software adapter.
    ///
    /// One instance because that is the arrangement in which "another device" is a
    /// question `wgpu` can answer: resource identifiers are per-instance, so a texture
    /// from a *second instance* is not a foreign resource to wgpu-core but a
    /// non-existent one, and looking it up panics inside `wgpu` before any error scope
    /// is consulted. That is a property of `wgpu` 30 rather than of this crate — the
    /// same panic is what `Target::Texture` has always had for the same input — and it
    /// is why the hoisted-instance constructors (`Device::headless_with_instance`,
    /// `Device::for_surface_with_instance`) are the ones a host with two devices wants.
    fn two_devices() -> (Device, Device) {
        let instance = crate::startup::create_instance();
        let options = Options {
            adapter: Some("llvmpipe".into()),
            ..Options::default()
        };
        let first = Device::headless_with_instance(&instance, &options)
            .expect("llvmpipe is present wherever this suite runs");
        let second = Device::headless_with_instance(&instance, &options)
            .expect("a second device on the same instance");
        (first, second)
    }

    /// A texture satisfying the whole layer contract, on `device`.
    fn layer_texture(device: &Device) -> wgpu::Texture {
        let (gpu, _) = device.wgpu();
        gpu.create_texture(&wgpu::TextureDescriptor {
            label: Some("a layer"),
            size: wgpu::Extent3d {
                width: 32,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: WARM_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    }

    /// Bind `texture` as a layer of `device`, and say what happened.
    fn bind_on(device: &Device, texture: &wgpu::Texture) -> Result<(), RenderError> {
        let (gpu, queue) = device.wgpu();
        let sampler = gpu.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("a filtering sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let layout = device.pipelines().present_layout();
        bind_layers(
            gpu,
            queue,
            &sampler,
            &layout,
            &[Layer {
                texture,
                placement: Affine::IDENTITY,
                filter: ImageFilter::Linear,
            }],
        )
        .map(drop)
    }

    /// **A layer from another device is refused by name, not by a panic.**
    ///
    /// `wgpu::Texture` does not say which device made it (version 30 offers its size,
    /// format, usage and shape and nothing else), so the question is put to `wgpu`
    /// itself — inside a validation error scope, because the default handler for an
    /// uncaptured error is a panic on whatever thread the host put the presenter on.
    /// This is the one refusal in the layer contract that cannot be checked from
    /// outside, and it is the one the caller's arrangement makes reachable: two devices
    /// exist in that process, and a texture is a handle that says nothing about which.
    #[test]
    fn a_layer_from_another_device_is_a_named_refusal() {
        let (mine, other) = two_devices();
        let stranger = layer_texture(&other);
        match bind_on(&mine, &stranger) {
            Err(RenderError::LayerRefused {
                index,
                reason: LayerProblem::Unbindable { detail },
            }) => {
                assert_eq!(index, 0);
                assert!(!detail.is_empty(), "the refusal carries what wgpu said");
            }
            other => panic!("expected a named layer refusal, got {other:?}"),
        }
    }

    /// The accepting direction, without which the assertion above could pass by
    /// refusing every texture (ADR 0052).
    #[test]
    fn a_layer_from_this_device_binds() {
        let (mine, _) = two_devices();
        let texture = layer_texture(&mine);
        bind_on(&mine, &texture).expect("a texture this device made is bindable");
    }

    /// The host's bytes are `struct Params` of `present.wgsl`, field for field, at the
    /// offsets WGSL §14.4.4 and §14.4.6 put them at — the gate every uniform writer in
    /// this crate carries, because `wgpu` checks a uniform's size and never its layout.
    ///
    /// Each field gets a distinct value so that two of one width exchanged fails.
    #[test]
    fn the_present_params_are_written_where_the_shader_reads_them() {
        let inverse = Affine {
            a: 1.5,
            b: 2.5,
            c: 3.5,
            d: 4.5,
            e: 5.5,
            f: 6.5,
        };
        let bytes = params_bytes(inverse, ImageFilter::Linear, [640.0, 480.0]);
        check(
            shaders::PRESENT,
            "Params",
            &bytes,
            &[
                ("inv0", Lane::Vec4([1.5, 2.5, 3.5, 4.5])),
                ("inv1", Lane::Vec4([5.5, 6.5, 1.0, 0.0])),
                ("source", Lane::Vec2([640.0, 480.0])),
            ],
        );
    }

    /// The filter is a number the shader compares against 0.5, so which number each
    /// variant is is part of the contract rather than an encoding detail.
    #[test]
    fn nearest_is_zero_and_linear_is_one() {
        let nearest = params_bytes(Affine::IDENTITY, ImageFilter::Nearest, [1.0, 1.0]);
        let linear = params_bytes(Affine::IDENTITY, ImageFilter::Linear, [1.0, 1.0]);
        assert_eq!(&nearest[24..28], &0.0_f32.to_le_bytes());
        assert_eq!(&linear[24..28], &1.0_f32.to_le_bytes());
    }
}

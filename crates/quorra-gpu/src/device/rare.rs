//! The bindings of the two rare-case lanes: an image placement (ISO 32000-2 §8.9.5)
//! and a shading (§8.7.4.5), each one quad driven by a uniform.
//!
//! The brief's §0 premise is that most of a page is glyph outlines and axis-aligned
//! rectangles, so these two are deliberately not a third instance stream (ADR 0011): a
//! page may place a handful of images and no shading at all, and a lane costs more to
//! keep in step than a uniform costs to write. `crate::encode::rare` is the half of
//! that decision that says what to draw; this is the half that binds it.
//!
//! They sit apart from `super::binds` for the reason they exist apart at all — the
//! compositor's passes run on every frame with a group in it, and these run on the
//! pages that happen to carry a picture.
//!
//! Which texture a paint id names is `super::resident`'s answer. A miss here is still
//! an `Err` naming the id: the ids were validated during encode, and a binding that
//! trusted that invariant silently would be a wrong page where a refusal was owed.

use quorra_scene::{ImageId, MeshId, RampId};

use super::Device;
use crate::compose::Region;
use crate::encode::{ImageOp, PaintSource, ShadedOp};
use crate::error::RenderError;
use crate::mask::MaskPlacement;

impl Device {
    /// The image quad's uniform + bind group for one `ImageOp` (ISO 32000-2
    /// §8.9.5; layout mirrored in `image.wgsl`'s `Params`).
    pub(crate) fn image_bind(
        &self,
        op: &ImageOp,
        region: Region,
        mask: (&wgpu::TextureView, MaskPlacement),
        scratch: &wgpu::TextureView,
    ) -> Result<wgpu::BindGroup, RenderError> {
        // The placement's resolved variant (ADR 0089): the reduced texture where the
        // encode named factors, the image's own samples otherwise.
        let looked_up = match op.reduced {
            Some((fx, fy)) => self.reduced_textures.get(&(op.image, fx, fy)),
            None => self.image_textures.get(&op.image),
        };
        let Some((_, image_view)) = looked_up else {
            return Err(RenderError::UnknownImage {
                image: ImageId(op.image),
            });
        };
        let uniform = self.quad_uniform(
            "quorra image params",
            &image_params_bytes(op, region, mask.1),
        );
        let layout = self.pipelines.image_layout();
        Ok(self.gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra image"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(image_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(mask.0),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(scratch),
                },
            ],
        }))
    }

    /// The shading quad's uniform + bind group for one `ShadedOp` (ISO 32000-2
    /// §8.7.4.5; layout mirrored in `shading.wgsl`'s `Params`).
    pub(crate) fn shaded_bind(
        &self,
        op: &ShadedOp,
        region: Region,
        scratch: &wgpu::TextureView,
        mask: (&wgpu::TextureView, MaskPlacement),
    ) -> Result<wgpu::BindGroup, RenderError> {
        let paint_view = match op.paint {
            PaintSource::Ramp(id) => {
                let Some((_, view)) = self.ramp_textures.get(&id) else {
                    return Err(RenderError::UnknownRamp { ramp: RampId(id) });
                };
                view
            }
            PaintSource::Mesh(id) => {
                let Some((_, view)) = self.mesh_textures.get(&id) else {
                    return Err(RenderError::UnknownMesh { mesh: MeshId(id) });
                };
                view
            }
        };
        let uniform = self.quad_uniform(
            "quorra shading params",
            &shading_params_bytes(op, region, mask.1),
        );
        let layout = self.pipelines.shading_layout();
        Ok(self.gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra shading"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(paint_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(scratch),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(mask.0),
                },
            ],
        }))
    }
}

/// The 144 bytes `image.wgsl`'s `Params` reads, in its order (ISO 32000-2 §8.9.5).
#[allow(clippy::arithmetic_side_effects)] // fixed-layout offsets in a 144-byte array
#[allow(clippy::cast_precision_loss)] // target sizes are far below 2^24
fn image_params_bytes(op: &ImageOp, region: Region, mask: MaskPlacement) -> [u8; 144] {
    let mut bytes = [0_u8; 144];
    let mut put = |at: usize, v: f32| bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
    for (i, v) in op.inv.iter().enumerate() {
        put(i * 4, *v); // inv0 then inv1.xy
    }
    put(24, op.alpha);
    put(28, if op.linear { 1.0 } else { 0.0 });
    for (i, v) in op.image_rect.iter().enumerate() {
        put(32 + i * 4, *v);
    }
    for (i, v) in op.dest.iter().enumerate() {
        put(48 + i * 4, *v);
    }
    for (i, v) in op.clip.iter().enumerate() {
        put(64 + i * 4, *v);
    }
    let origin = op.residue_origin.unwrap_or([0.0, 0.0]);
    put(80, origin[0]);
    put(84, origin[1]);
    put(
        88,
        if op.residue_origin.is_some() {
            1.0
        } else {
            0.0
        },
    );
    put(92, if op.axis_aligned { 1.0 } else { 0.0 });
    put(96, region.width as f32);
    put(100, region.height as f32);
    // The attachment's device origin (ADR 0036), which `vs_main` subtracts and
    // `fs_main` adds back.
    put(104, region.x as f32);
    put(108, region.y as f32);
    bytes[112..144].copy_from_slice(&mask.bytes());
    bytes
}

/// The 176 bytes `shading.wgsl`'s `Params` reads, in its order (§8.7.4.5).
#[allow(clippy::arithmetic_side_effects)] // fixed-layout offsets in a 176-byte array
#[allow(clippy::cast_precision_loss)] // extend bits ≤ 3; sizes far below 2^24
fn shading_params_bytes(op: &ShadedOp, region: Region, mask: MaskPlacement) -> [u8; 176] {
    let mut bytes = [0_u8; 176];
    let mut put = |at: usize, v: f32| bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
    for (i, v) in op.inv.iter().enumerate() {
        put(i * 4, *v); // inv0 then inv1.xy
    }
    put(24, op.kind_word);
    put(28, op.extend_bits as f32);
    for (i, v) in op.geo0.iter().enumerate() {
        put(32 + i * 4, *v);
    }
    for (i, v) in op.geo1.iter().enumerate() {
        put(48 + i * 4, *v);
    }
    for (i, v) in op.dest.iter().enumerate() {
        put(64 + i * 4, *v);
    }
    let origin = op.coverage_origin.unwrap_or([0.0, 0.0]);
    put(80, origin[0]);
    put(84, origin[1]);
    put(
        88,
        if op.coverage_origin.is_some() {
            1.0
        } else {
            0.0
        },
    );
    for (i, v) in op.coverage_rect.iter().enumerate() {
        put(96 + i * 4, *v);
    }
    for (i, v) in op.clip.iter().enumerate() {
        put(112 + i * 4, *v);
    }
    put(128, region.width as f32);
    put(132, region.height as f32);
    // The attachment's device origin (ADR 0036).
    put(136, region.x as f32);
    put(140, region.y as f32);
    bytes[144..176].copy_from_slice(&mask.bytes());
    bytes
}

/// The two rare lanes' uniforms against the WGSL structs they mirror.
///
/// Both lanes pack four numbers of different meanings into one `vec4f` — an inverse
/// transform's last two coefficients beside a constant alpha and a filter flag — and a
/// lane written into the wrong half of one of those is invisible to `wgpu`, whose
/// validation sees 144 bytes either way.
#[cfg(test)]
mod tests {
    use super::{
        ImageOp, MaskPlacement, PaintSource, Region, ShadedOp, image_params_bytes,
        shading_params_bytes,
    };
    use crate::encode::DrawStyle;
    use crate::shaders;
    use crate::shaders::layout::{Lane, check};

    /// The mask placement every quad lane ends with (ADR 0037), as the two `vec4f` the
    /// shaders read it as.
    const MASK: MaskPlacement = MaskPlacement {
        origin: [61.0, 62.0],
        size: [63.0, 64.0],
        outside: 0.75,
    };

    /// The region every one of these tests draws into: four numbers no field shares.
    const REGION: Region = Region {
        x: 71,
        y: 72,
        width: 73,
        height: 74,
    };

    #[test]
    fn the_image_uniform_is_images_params() {
        let op = ImageOp {
            image: 0,
            inv: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            image_rect: [11.0, 12.0, 13.0, 14.0],
            dest: [21.0, 22.0, 23.0, 24.0],
            clip: [31.0, 32.0, 33.0, 34.0],
            residue_origin: Some([41.0, 42.0]),
            reduced: None,
            axis_aligned: true,
            alpha: 0.5,
            linear: true,
            style: DrawStyle::Over,
            mask: None,
        };
        let bytes = image_params_bytes(&op, REGION, MASK);
        check(
            shaders::IMAGE,
            "Params",
            &bytes,
            &[
                ("inv0", Lane::Vec4([1.0, 2.0, 3.0, 4.0])),
                // §8.3.3's e and f, then §11.6.4.4's constant alpha and the resolved filter.
                ("inv1", Lane::Vec4([5.0, 6.0, 0.5, 1.0])),
                ("image_rect", Lane::Vec4([11.0, 12.0, 13.0, 14.0])),
                ("dest", Lane::Vec4([21.0, 22.0, 23.0, 24.0])),
                ("clip", Lane::Vec4([31.0, 32.0, 33.0, 34.0])),
                // Scratch origin, then "there is a residue", then "axes preserved".
                ("coverage", Lane::Vec4([41.0, 42.0, 1.0, 1.0])),
                ("target_size", Lane::Vec2([73.0, 74.0])),
                ("origin", Lane::Vec2([71.0, 72.0])),
                ("mask_rect", Lane::Vec4([61.0, 62.0, 63.0, 64.0])),
                ("mask_outside", Lane::Vec4([0.75, 0.0, 0.0, 0.0])),
            ],
        );
    }

    #[test]
    fn the_shading_uniform_is_shadings_params() {
        let op = ShadedOp {
            paint: PaintSource::Ramp(0),
            inv: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            kind_word: 1.0,
            extend_bits: 3,
            geo0: [11.0, 12.0, 13.0, 14.0],
            geo1: [15.0, 16.0, 0.0, 0.0],
            dest: [21.0, 22.0, 23.0, 24.0],
            coverage_origin: Some([41.0, 42.0]),
            coverage_rect: [51.0, 52.0, 53.0, 54.0],
            clip: [31.0, 32.0, 33.0, 34.0],
            style: DrawStyle::Over,
            mask: None,
        };
        let bytes = shading_params_bytes(&op, REGION, MASK);
        check(
            shaders::SHADING,
            "Params",
            &bytes,
            &[
                ("inv0", Lane::Vec4([1.0, 2.0, 3.0, 4.0])),
                // §8.3.3's e and f, then the shading kind and §8.7.4.5.2's extend bits.
                ("inv1", Lane::Vec4([5.0, 6.0, 1.0, 3.0])),
                ("geo0", Lane::Vec4([11.0, 12.0, 13.0, 14.0])),
                ("geo1", Lane::Vec4([15.0, 16.0, 0.0, 0.0])),
                ("dest", Lane::Vec4([21.0, 22.0, 23.0, 24.0])),
                ("coverage", Lane::Vec4([41.0, 42.0, 1.0, 0.0])),
                ("coverage_rect", Lane::Vec4([51.0, 52.0, 53.0, 54.0])),
                ("clip", Lane::Vec4([31.0, 32.0, 33.0, 34.0])),
                ("target_size", Lane::Vec2([73.0, 74.0])),
                ("origin", Lane::Vec2([71.0, 72.0])),
                ("mask_rect", Lane::Vec4([61.0, 62.0, 63.0, 64.0])),
                ("mask_outside", Lane::Vec4([0.75, 0.0, 0.0, 0.0])),
            ],
        );
    }
}

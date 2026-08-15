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
    #[allow(clippy::arithmetic_side_effects)] // fixed-layout offsets in a 144-byte array
    #[allow(clippy::cast_precision_loss)] // target sizes are far below 2^24
    pub(crate) fn image_bind(
        &self,
        op: &ImageOp,
        region: Region,
        mask: (&wgpu::TextureView, MaskPlacement),
        scratch: &wgpu::TextureView,
    ) -> Result<wgpu::BindGroup, RenderError> {
        let (width, height) = (region.width, region.height);
        let Some((_, image_view)) = self.image_textures.get(&op.image) else {
            return Err(RenderError::UnknownImage {
                image: ImageId(op.image),
            });
        };
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
        put(96, width as f32);
        put(100, height as f32);
        // The attachment's device origin (ADR 0036), which `vs_main` subtracts and
        // `fs_main` adds back.
        put(104, region.x as f32);
        put(108, region.y as f32);
        bytes[112..144].copy_from_slice(&mask.1.bytes());
        let uniform = self.quad_uniform("quorra image params", &bytes);
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
    #[allow(clippy::arithmetic_side_effects)] // fixed-layout offsets in a 176-byte array
    #[allow(clippy::cast_precision_loss)] // extend bits ≤ 3; sizes far below 2^24
    pub(crate) fn shaded_bind(
        &self,
        op: &ShadedOp,
        region: Region,
        scratch: &wgpu::TextureView,
        mask: (&wgpu::TextureView, MaskPlacement),
    ) -> Result<wgpu::BindGroup, RenderError> {
        let (width, height) = (region.width, region.height);
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
        put(128, width as f32);
        put(132, height as f32);
        // The attachment's device origin (ADR 0036).
        put(136, region.x as f32);
        put(140, region.y as f32);
        bytes[144..176].copy_from_slice(&mask.1.bytes());
        let uniform = self.quad_uniform("quorra shading params", &bytes);
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

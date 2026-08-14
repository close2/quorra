//! The frame's soft masks: realised first, in id order, and looked up by whoever binds
//! one.
//!
//! ISO 32000-2 §11.5 makes a soft mask a transparency group rendered on its own and
//! reduced to mask values, so this runs before any content pass — a mask may reference
//! only masks defined before it, which is what makes one pass in id order enough. Each
//! group renders through the ordinary plan machinery and `reduce.wgsl` turns it into R8.
//!
//! The lookup at the end is the other half: five sites in the frame bind a mask, and
//! every one of them may be binding *no* mask, which is a texture too.

use crate::error::RenderError;
use crate::mask::{MaskPlacement, Realised};
use crate::pipeline::Kind;

use super::Executor;

impl Executor<'_> {
    /// Realise every used soft mask, in id order (§11.5), into R8 views.
    pub(crate) fn realise_masks(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
    ) -> Result<(), RenderError> {
        for index in 0..self.encoded.mask_plans.len() {
            let Some(plan) = &self.encoded.mask_plans[index] else {
                continue;
            };
            // A soft mask's group renders on its own, onto transparency (§11.5), at its
            // own plan's rectangle like every other layer (ADR 0037).
            //
            // **The reduce below needs no origin**: it reads the group at the fragment's
            // own position and writes the R8 at the same one, and the two textures are
            // the same size, so they map 1:1 wherever that rectangle sits. What the
            // frame's five sampling sites need is where the R8 *is* and what surrounds
            // it, which is the placement below.
            let group = self.render_plan(recorder, plan.root.saturating_add(1), None)?;
            let region = group.region();
            let group_view = group.view();
            let mask_texture = self.device.create_internal_texture(
                "quorra soft mask",
                region.width,
                region.height,
                wgpu::TextureFormat::R8Unorm,
            );
            let mask_view = mask_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind = self.device.reduce_bind(plan, &group_view);
            let (pipeline, compiled) = self
                .device
                .pipelines()
                .get(Kind::Reduce, wgpu::TextureFormat::R8Unorm)?;
            if let Some(duration) = compiled {
                self.phases.push(("pipeline compile (first use)", duration));
            }
            let mut pass = recorder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("quorra reduce"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &mask_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: self.pass_stamp(),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.scissor_pass(&mut pass, region, None);
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            // The reduce has read it; the mask's own R8 is what outlives this loop.
            self.pool.release(group.texture);
            if let Some(slot) = self.masks.get_mut(index) {
                *slot = Some(Realised {
                    view: mask_view,
                    placement: MaskPlacement {
                        #[allow(clippy::cast_precision_loss)] // extents are exact in f32
                        origin: [region.x as f32, region.y as f32],
                        #[allow(clippy::cast_precision_loss)]
                        size: [region.width as f32, region.height as f32],
                        outside: crate::mask::transparent_value(plan),
                    },
                });
            }
        }
        Ok(())
    }

    /// The mask an op names: the view to bind, and where its texels are.
    ///
    /// An op that names no mask — or one the frame did not realise — gets the 1 × 1
    /// stand-in and [`MaskPlacement::ABSENT`], whose every sample is outside it and so
    /// admits everything. The view still has to be bound: a layout entry is not optional.
    pub(super) fn mask_for(&self, mask: Option<u32>) -> (&wgpu::TextureView, MaskPlacement) {
        let realised = mask
            .and_then(|index| self.masks.get(index as usize))
            .and_then(Option::as_ref);
        realised.map_or((&self.dummy_view, MaskPlacement::ABSENT), |realised| {
            (&realised.view, realised.placement)
        })
    }
}

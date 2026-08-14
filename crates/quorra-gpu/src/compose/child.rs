//! One finished child, composited onto the plan that accumulates it — ISO 32000-2
//! §11.3.6, and the two passes it takes on a device that cannot read what it writes.
//!
//! `composite.wgsl` needs the backdrop and the child at once and writes the backdrop's
//! own attachment, so the pixels it is about to cover are copied out first. That copy
//! is why a plan needs one texture rather than a ping-pong pair (ADR 0038), and its
//! size — `child ∩ parent` rather than the whole plan — is why the pair below takes two
//! regions everywhere.

use crate::encode::ChildOp;
use crate::error::RenderError;
use crate::pipeline::Kind;

use super::{Executor, Region, Rendered, view_of};

impl Executor<'_> {
    /// Composite one finished child onto the plan accumulating it (§11.3.6).
    ///
    /// Two passes, and the first is what lets there be one texture per plan rather than
    /// two (ADR 0038): the composite cannot read the attachment it writes, so the pixels
    /// it is about to cover are copied out — **at the size of `child ∩ parent`**, because
    /// that is the whole of what it writes. Outside the child's own rectangle every branch
    /// of `composite.wgsl` collapses to the backdrop it read, so those pixels are already
    /// what the pass would put there.
    ///
    /// A child that meets its parent nowhere composites to nothing: the clip that shrank
    /// the parent's bounds is the same clip whose coverage the pass would multiply by, and
    /// it is zero everywhere the child could have contributed. **By the time a child is
    /// rendered it is too late to save anything by discovering that** — the encoder drops
    /// such a child before it becomes an op at all (ADR 0041), which is where the clip
    /// that emptied it is known.
    pub(super) fn composite_child(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
        accumulator: &wgpu::TextureView,
        region: Region,
        child: &Rendered,
        op: &ChildOp,
    ) -> Result<(), RenderError> {
        let Some(onto) = region.meet(child.region()) else {
            return Ok(());
        };
        let copy = self.pool.acquire(self.device, onto.width, onto.height);
        let copy_view = view_of(&copy);
        #[allow(clippy::cast_precision_loss)] // extents are exact in f32
        let from = [
            onto.x.saturating_sub(region.x) as f32,
            onto.y.saturating_sub(region.y) as f32,
        ];
        self.copy_pass(
            recorder,
            "quorra composite backdrop",
            (accumulator, region),
            (&copy_view, onto),
            from,
        )?;
        self.composite_pass(
            recorder,
            accumulator,
            region,
            (&copy_view, onto),
            (&child.view(), child.region()),
            op,
        )?;
        self.pool.release(copy);
        Ok(())
    }

    /// One composite pass: `accumulator = child over/blended-onto backdrop` per §11.3.6.
    ///
    /// The attachment is the plan's own accumulator and the pass is **scissored to
    /// `onto`** — the part of it the child can reach (ADR 0038). `backdrop` holds the copy
    /// of exactly those pixels, made before this pass because a pass cannot read what it
    /// writes. The load op is `Load` for the same reason: everything outside the scissor
    /// is already what this pass would have written there.
    fn composite_pass(
        &mut self,
        recorder: &mut wgpu::CommandEncoder,
        accumulator: &wgpu::TextureView,
        region: Region,
        backdrop: (&wgpu::TextureView, Region),
        child: (&wgpu::TextureView, Region),
        op: &ChildOp,
    ) -> Result<(), RenderError> {
        let mask = self.mask_for(op.mask);
        let scratch = self.scratch_view.as_ref().unwrap_or(&self.dummy_view);
        let bind = self
            .device
            .composite_bind(op, region, backdrop, child, mask, scratch);
        let (pipeline, compiled) = self
            .device
            .pipelines()
            .get(Kind::Composite, wgpu::TextureFormat::Rgba8Unorm)?;
        if let Some(duration) = compiled {
            self.phases.push(("pipeline compile (first use)", duration));
        }
        let stamp = self.pass_stamp();
        let mut pass = recorder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("quorra composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: accumulator,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: stamp,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.scissor_pass(&mut pass, region, Some(backdrop.1));
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}

//! Phase 3: one frame's passes, recorded into a single command encoder and submitted.
//!
//! Everything a pass needs exists before this module runs — the target is bound
//! (`super::bound`), the buffers are staged (`super::staging`), the bindings are
//! `super::binds`'s and the passes themselves are `crate::compose`'s. What is decided
//! here is the **route**, and [`Route`] names all four of them:
//!
//! - **patched** — the frame renders into the root pair with every pass scissored to
//!   the damage bounding box, and exactly the damage rectangles are replaced on the
//!   caller's retained texture (ADR 0012);
//! - **flat** — a root with no children and no masks draws straight into the target,
//!   which is the fast path M1 measured and nothing since has been allowed to make
//!   slower;
//! - **layered** — masks realise first, the plan tree renders bottom-up, and the root
//!   is blitted to the target;
//! - **untouched** — a damage list whose every rectangle fell outside the target, which
//!   touches no pixel at all. That is honouring the list exactly rather than failing to
//!   draw, and the distinction is why it is a named route and not a fallthrough.
//!
//! The two timestamps that make `Timings::execute` mean first pass to last are
//! resolved here as well (ADR 0031), because the resolve has to be recorded into the
//! same encoder as the passes it measures.

use std::collections::HashMap;
use std::time::Duration;

use super::Device;
use super::bound::Bound;
use super::damage::DamagePlan;
use super::staging::Upload;
use crate::compose::{self, Executor, PassLoad, Region};
use crate::encode::Encoded;
use crate::error::RenderError;
use crate::layers::LayerPool;
use crate::timing::PassQuery;

/// One frame's per-pass durations and one-off costs.
pub(super) type FramePhases = Vec<(&'static str, Duration)>;

/// The route one frame's content takes to the target.
#[derive(Clone, Copy)]
enum Route<'a> {
    /// The patched path (ADR 0012): render the frame into the root pair, every pass
    /// scissored to the damage bounding box, then replace exactly the damage
    /// rectangles on the caller's retained texture.
    Patch {
        bbox: [u32; 4],
        rects: &'a [[u32; 4]],
    },
    /// Every damage rect fell outside the target: nothing visible changed, and
    /// honouring the list exactly means touching no pixel at all.
    Untouched,
    /// A root with no children and no masks, drawn straight into the target — the fast
    /// path M1 measured.
    Flat,
    /// Masks first, then the plan tree bottom-up, then the root onto the target.
    Layered,
}

impl<'a> Route<'a> {
    /// Which of the four a frame takes, from its damage plan and whether its root is
    /// flat. The order of the arms is the order of the questions: a patched frame
    /// renders through the root pair even when it is flat.
    fn of(damage: &'a DamagePlan, flat: bool) -> Self {
        match damage {
            DamagePlan::Patch { bbox, rects } if !rects.is_empty() => {
                Route::Patch { bbox: *bbox, rects }
            }
            DamagePlan::Patch { .. } => Route::Untouched,
            DamagePlan::Full if flat => Route::Flat,
            DamagePlan::Full => Route::Layered,
        }
    }

    /// The rectangle every internal pass is scissored to, where the route has one.
    fn scissor(&self) -> Option<[u32; 4]> {
        match self {
            Route::Patch { bbox, .. } => Some(*bbox),
            _ => None,
        }
    }
}

/// Record one frame's content along its route, into `recorder`.
fn record_content(
    executor: &mut Executor<'_>,
    recorder: &mut wgpu::CommandEncoder,
    route: Route<'_>,
    target: (&wgpu::TextureView, wgpu::TextureFormat),
) -> Result<(), RenderError> {
    let (target_view, target_format) = target;
    match route {
        Route::Patch { rects, .. } => {
            executor.realise_masks(recorder)?;
            let root = executor.render_plan(recorder, 0, None)?;
            executor.patch_to_target(
                recorder,
                &root.view(),
                root.region(),
                target_view,
                target_format,
                rects,
            )?;
        }
        Route::Untouched => {}
        Route::Flat => {
            // is_flat checked: a flat root holds drawable ops only.
            let root_ops = compose::run_ops(&executor.encoded.root.ops);
            executor.draw_pass(
                recorder,
                target_view,
                target_format,
                PassLoad::Clear,
                &root_ops,
            )?;
        }
        Route::Layered => {
            executor.realise_masks(recorder)?;
            let root = executor.render_plan(recorder, 0, None)?;
            executor.blit_to_target(
                recorder,
                &root.view(),
                root.region(),
                target_view,
                target_format,
            )?;
        }
    }
    Ok(())
}

impl Device {
    /// Phase 3: the whole device side of one frame — mask realisation, layers,
    /// composites, the flat fast path, timestamps — recorded and submitted.
    pub(super) fn run_frame(
        &mut self,
        encoded: &Encoded,
        bound: &Bound<'_>,
        upload: Upload,
        query: Option<&PassQuery>,
        damage: &DamagePlan,
    ) -> Result<(Duration, FramePhases, u32), RenderError> {
        let width = bound.texture().width();
        let height = bound.texture().height();
        let mut recorder = self
            .gpu
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("quorra frame"),
            });
        let route = Route::of(damage, Executor::is_flat(encoded));
        let dummy_view = self.ensure_dummy();
        let mask_count = encoded.mask_plans.len();
        // Taken, not borrowed: it belongs to the first frame of its size and to no other.
        let warmed = match self.warmed_layer.take() {
            Some((w, h, pair)) if (w, h) == (width, height) => Some(pair),
            _ => None,
        };
        let mut executor = Executor {
            device: self,
            encoded,
            width,
            height,
            // Empty unless a host called `warm_for` for this size, which puts one pair
            // in it: a plan's pair is otherwise created on its first acquire, so a flat
            // frame creates none at all (ADR 0020).
            pool: LayerPool::warmed(warmed),
            masks: (0..mask_count).map(|_| None).collect(),
            rect_buffer: upload.rect_instances,
            quad_buffer: upload.quad_instances,
            lane_binds: HashMap::new(),
            scratch_view: upload.scratch_view.as_ref().map(|(_, view)| view.clone()),
            dummy_view,
            atlas_view: self.atlas_texture.as_ref().map(|(_, view)| view.clone()),
            first_pass_stamped: false,
            query,
            phases: Vec::new(),
            scissor: route.scissor(),
            region: Region::whole(width, height),
        };
        let target_view = bound
            .texture()
            .create_view(&wgpu::TextureViewDescriptor::default());
        let target_format = bound.texture().format();
        record_content(
            &mut executor,
            &mut recorder,
            route,
            (&target_view, target_format),
        )?;
        executor.end_stamp(&mut recorder, &target_view);
        let layer_textures = u32::try_from(executor.pool.peak()).unwrap_or(u32::MAX);
        let phases = std::mem::take(&mut executor.phases);
        drop(executor);
        if let Some(q) = query {
            recorder.resolve_query_set(&q.set, 0..2, &q.resolve, 0);
            recorder.copy_buffer_to_buffer(&q.resolve, 0, &q.map, 0, 16);
        }
        let execute_wall = compose::submit_and_wait(self, recorder)?;
        Ok((execute_wall, phases, layer_textures))
    }
}

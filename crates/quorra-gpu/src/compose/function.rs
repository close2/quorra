//! The function lane's pass: its uniform, its bind group, and the pipeline generated for
//! the program it names (ADR 0053).
//!
//! One responsibility: **turn one [`FunctionOp`] into the two things a draw needs** — a
//! bind group and a pipeline — neither of which can be made while the pass borrows the
//! recorder, which is why this is called from `draw::prepare_run` and not from the pass.
//!
//! It is the only place in the compositor that knows a paint can be a program. The pass
//! itself sets a pipeline and a bind group and draws four corners, exactly as it does for
//! an image or a ramp sweep.
//!
//! # The byte layout is a promise to a shader, and it is stated once
//!
//! [`uniform_bytes`] mirrors `shaders/function_lane.wgsl`'s `Params`, offset for offset.
//! The bind-group layout's `min_binding_size` is what makes a disagreement a validation
//! error rather than a wrong picture (`pipeline/layouts.rs` says so for every uniform in
//! the crate), so the two numbers that must agree — 208 here and 208 there — are checked
//! by `wgpu` on the first frame that draws one.

use std::sync::Arc;

use super::{Executor, Region};
use crate::encode::FunctionOp;
use crate::error::RenderError;
use crate::function::range_bounds;
use crate::mask::MaskPlacement;
use crate::pipeline::Style;

/// The uniform's size, in bytes. `function_lane.wgsl`'s `Params` and the bind-group
/// layout's `min_binding_size` are the other two places this number appears.
const UNIFORM_BYTES: usize = 208;

impl Executor<'_> {
    /// The bind group for one function quad: its parameters, the frame's scratch, and the
    /// active soft mask.
    ///
    /// Infallible, unlike the image and shading lanes' — and the difference says what a
    /// generated paint *is*. Those two resolve a texture the resource store may no longer
    /// hold; this one binds numbers the op already carries, and the only thing it needs
    /// from the store is the analysis, which [`Executor::function_pipelines`] asks for and
    /// refuses by name.
    pub(super) fn function_bind(
        &self,
        op: &FunctionOp,
        scratch: &wgpu::TextureView,
        mask: (&wgpu::TextureView, MaskPlacement),
    ) -> wgpu::BindGroup {
        let (gpu, _) = self.device.wgpu();
        let bytes = uniform_bytes(op, self.region, mask.1);
        let uniform = gpu.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quorra function params"),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (_, queue) = self.device.wgpu();
        queue.write_buffer(&uniform, 0, &bytes);
        let layout = self.device.pipelines().function_layout();
        gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra function"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(scratch),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(mask.0),
                },
            ],
        })
    }

    /// The generated pipelines this op draws with — one for an ordinary mark, the
    /// knockout pair for §11.4.6's stages — compiling each on first use.
    ///
    /// **This is the only place a page pays for a program's shader**, and it pays once per
    /// distinct program, style and target format: the cache is keyed by the program's
    /// content, so a page with a thousand fills of one shading compiles one (ADR 0053, and
    /// `Scene::cost`'s `function_programs` is what a caller compares against beforehand).
    ///
    /// # Errors
    ///
    /// [`RenderError::UnknownFunction`] for a program no longer resident, or
    /// [`RenderError::PipelineUnavailable`] naming what the adapter refused about the
    /// generated shader. A frame that cannot compile its paint is refused, never drawn
    /// without it.
    pub(super) fn function_pipelines(
        &mut self,
        op: &FunctionOp,
        format: wgpu::TextureFormat,
    ) -> Result<[Option<Arc<wgpu::RenderPipeline>>; 2], RenderError> {
        let program = op.program();
        let analysis = self
            .device
            .function_analysis(program)
            .ok_or(RenderError::UnknownFunction { program })?;
        let mut resolved = [None, None];
        for (slot, style) in resolved.iter_mut().zip(Style::of(op.style())) {
            let Some(style) = style else { continue };
            let (pipeline, compiled) = self
                .device
                .pipelines()
                .function_pipeline(analysis, op.range, style, format)?;
            if let Some(duration) = compiled {
                // Named as its own phase rather than folded into the fixed table's: what
                // it costs is a function of the program's length, so a frame that is slow
                // because a page brought a 482-instruction shading should say so.
                self.phases
                    .push(("function shader compile (first use)", duration));
            }
            *slot = Some(pipeline);
        }
        Ok(resolved)
    }
}

/// The 208 bytes `function_lane.wgsl`'s `Params` reads, in its order.
///
/// Written as a flat sequence of `f32` lanes with the struct's own field boundaries kept
/// visible in the comments, which is the same shape `device.rs` writes the image and
/// shading uniforms in — a table of offsets a reader can check against the WGSL by
/// counting.
#[allow(clippy::arithmetic_side_effects)] // fixed-layout offsets in a 208-byte array
#[allow(clippy::cast_precision_loss)] // target sizes are far below 2^24
fn uniform_bytes(op: &FunctionOp, region: Region, mask: MaskPlacement) -> [u8; UNIFORM_BYTES] {
    let mut bytes = [0_u8; UNIFORM_BYTES];
    let mut put = |at: usize, v: f32| bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
    for (i, v) in op.inv.iter().enumerate() {
        put(i * 4, *v); // inv0, then inv1.xy
    }
    for (i, v) in op.domain.iter().enumerate() {
        put(32 + i * 4, *v);
    }
    let (low, high) = range_bounds(op.range);
    for (i, v) in low.iter().enumerate() {
        put(48 + i * 4, *v);
    }
    for (i, v) in high.iter().enumerate() {
        put(64 + i * 4, *v);
    }
    for (i, v) in op.background.iter().enumerate() {
        put(80 + i * 4, *v);
    }
    for (i, v) in op.placement.dest.iter().enumerate() {
        put(96 + i * 4, *v);
    }
    let origin = op.placement.coverage_origin.unwrap_or([0.0, 0.0]);
    put(112, origin[0]);
    put(116, origin[1]);
    put(
        120,
        if op.placement.coverage_origin.is_some() {
            1.0
        } else {
            0.0
        },
    );
    for (i, v) in op.placement.coverage_rect.iter().enumerate() {
        put(128 + i * 4, *v);
    }
    for (i, v) in op.placement.clip.iter().enumerate() {
        put(144 + i * 4, *v);
    }
    put(160, region.width as f32);
    put(164, region.height as f32);
    // The attachment's device origin (ADR 0036).
    put(168, region.x as f32);
    put(172, region.y as f32);
    bytes[176..208].copy_from_slice(&mask.bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use quorra_scene::FnRange;

    use super::{FunctionOp, MaskPlacement, Region, uniform_bytes};
    use crate::shaders;
    use crate::shaders::layout::{Lane, check};

    /// The generated lane's uniform against the fixed half of the shader it is appended
    /// to (ADR 0053).
    ///
    /// The module comment above says the 208 that `wgpu` checks is the size, and that a
    /// disagreement about it is a validation error; this is the other half of that
    /// sentence — where inside the 208 each number goes, which `wgpu` never looks at.
    /// `range_low` and `range_high` are the pair at risk: two `vec4f` of the same width
    /// holding §7.10.1's `Range` bounds, whose exchange would clamp every output to the
    /// wrong end of its interval and still draw.
    #[test]
    fn the_function_uniform_is_the_lanes_params() {
        let op = FunctionOp::from_parts(
            [1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            [11.0, 12.0, 13.0, 14.0],
            FnRange::Rgb([[0.1, 0.7], [0.2, 0.8], [0.3, 0.9]]),
            [0.25, 0.5, 0.75, 1.0],
            (
                [21.0, 22.0, 23.0, 24.0],
                Some([41.0, 42.0]),
                [51.0, 52.0, 53.0, 54.0],
                [31.0, 32.0, 33.0, 34.0],
            ),
        );
        let region = Region {
            x: 71,
            y: 72,
            width: 73,
            height: 74,
        };
        let mask = MaskPlacement {
            origin: [61.0, 62.0],
            size: [63.0, 64.0],
            outside: 0.75,
        };
        check(
            shaders::FUNCTION_LANE,
            "Params",
            &uniform_bytes(&op, region, mask),
            &[
                ("inv0", Lane::Vec4([1.0, 2.0, 3.0, 4.0])),
                // §8.3.3's e and f; the lane needs no third and fourth number here.
                ("inv1", Lane::Vec4([5.0, 6.0, 0.0, 0.0])),
                ("domain", Lane::Vec4([11.0, 12.0, 13.0, 14.0])),
                // §7.10.1's `Range`, low bounds then high, one lane per component and
                // the fourth unused — a `DeviceRGB` program emits three.
                ("range_low", Lane::Vec4([0.1, 0.2, 0.3, 0.0])),
                ("range_high", Lane::Vec4([0.7, 0.8, 0.9, 0.0])),
                ("background", Lane::Vec4([0.25, 0.5, 0.75, 1.0])),
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

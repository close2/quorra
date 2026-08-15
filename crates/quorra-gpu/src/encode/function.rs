//! The function lane: where a §7.10.5 program's placement is decided (ADR 0053).
//!
//! One responsibility: **turn a `Paint::Function` into a quad the device can draw.** It is
//! the shading lane's sibling and shares its arithmetic through [`QuadPlacement`] — the
//! destination rectangle, the coverage source, the clip — because those belong to the
//! *mark* and are the same whether the colour is swept from a ramp or computed per
//! fragment. What is here is the part that is not the same:
//!
//! - the program's identity, and the `Range` it is generated under, which together are the
//!   pipeline's key;
//! - §8.7.4.5.2's `Domain` and `Background`, which the shader needs as numbers;
//! - the inverse of §8.7.4.5.2's `Matrix` composed with the viewport, which is what carries
//!   a device pixel back into the shading's own space.
//!
//! # Two refusals belong here and nowhere else
//!
//! A program that is not resident, and a `Range` the resident program cannot fill. The
//! first is the resource contract every family has; the second is the one question about a
//! function paint that *cannot* be answered before the program and the shading meet, which
//! is why [`Analysis::admits`] is public API a caller can ask early and a refusal here
//! rather than a black quad.
//!
//! Everything else about the program was settled at `Device::upload_function`: the frame
//! never asks whether a program can be lowered, only whether this shading may name it.

use quorra_scene::{FnRange, FunctionId, Paint};

use super::rare::QuadPlacement;
use super::{DrawStyle, Encoder};
use crate::error::RenderError;
use crate::function::{background_rgba, domain_bounds};
use crate::report::{Report, ReportKind};
use crate::resources::ResourceStore;

/// One §7.10.5 function draw: a single uniform-driven quad over a coverage source, whose
/// colour the device computes from the program named by `program`.
///
/// `range` travels rather than its resolved bounds, because it is half of the pipeline's
/// key: the generated shader differs between a one-component and a three-component range,
/// and `function::range_bounds` turns it into the shader's two `vec3`s at bind time.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FunctionOp {
    /// The resident program's raw id.
    pub program: u32,
    /// §7.10.1's `Range`, and with it the component count the shader is generated for.
    pub range: FnRange,
    /// Inverse of the shading-space → device transform, §8.3.3 coefficient order.
    pub inv: [f32; 6],
    /// §8.7.4.5.2's `Domain` as (min x, max x, min y, max y).
    pub domain: [f32; 4],
    /// §8.7.4.5.2's `Background`, straight-alpha; `vec4(0)` when there is none, which is
    /// the clause's "left unpainted".
    pub background: [f32; 4],
    /// Where the quad goes, what weights it, and under which style.
    pub placement: QuadPlacement,
}

#[cfg(test)]
impl FunctionOp {
    /// One op assembled from its parts, for the uniform-layout gate in
    /// `crate::compose::function`.
    ///
    /// That module writes this op into `function_lane.wgsl`'s `Params` and its test
    /// reads every field back at the offset the shader puts it, but it cannot build an
    /// op: [`QuadPlacement`] is `encode`'s own and is re-exported to nobody. An inherent
    /// constructor reaches through the private module the type sits in, where a `use`
    /// cannot, which is why this is a method rather than a fixture beside the test.
    ///
    /// `quad` is the placement's four uniform-carried numbers, in the order the shader
    /// reads them: the destination rectangle, the coverage tile's scratch origin if it
    /// has one, the analytic coverage rectangle, and the clip. The two the uniform does
    /// *not* carry are fixed here — a placement's style chooses a pipeline and its mask
    /// chooses a texture, and neither reaches a byte of `Params`.
    pub(crate) fn from_parts(
        inv: [f32; 6],
        domain: [f32; 4],
        range: FnRange,
        background: [f32; 4],
        quad: ([f32; 4], Option<[f32; 2]>, [f32; 4], [f32; 4]),
    ) -> Self {
        let (dest, coverage_origin, coverage_rect, clip) = quad;
        Self {
            program: 0,
            range,
            inv,
            domain,
            background,
            placement: QuadPlacement {
                dest,
                coverage_origin,
                coverage_rect,
                clip,
                style: DrawStyle::Over,
                mask: None,
            },
        }
    }
}

/// The shading-space half of a function paint, resolved once per command — the twin of
/// `ShadedGeometry`, and separate from it for the reason the module comment gives.
#[derive(Debug, Clone, Copy)]
pub(super) struct FunctionGeometry {
    program: u32,
    range: FnRange,
    domain: [f32; 4],
    background: [f32; 4],
    inv: [f32; 6],
}

impl Encoder<'_> {
    /// The shading-space geometry of a [`Paint::Function`], or `None` where a singular
    /// `Matrix` makes the shading unmappable.
    ///
    /// `None` cannot actually be reached through a validated scene — the scene boundary
    /// refuses a singular matrix by name (`SceneError::SingularFunctionMatrix`), because a
    /// fragment shader has to invert it and a collapsed domain names no point to evaluate.
    /// It is still asked here rather than assumed: the encoder takes a `Scene` and the
    /// alternative to a check is an `unwrap` on a caller's arithmetic.
    pub(super) fn function_geometry(
        &mut self,
        paint: Paint,
    ) -> Result<Option<FunctionGeometry>, RenderError> {
        let Paint::Function {
            program,
            domain,
            matrix,
            range,
            background,
        } = paint
        else {
            // The callers match on the variant before calling; a second arm here would be
            // a second answer to which paints this lane draws.
            unreachable!("function_geometry is called for Paint::Function only")
        };
        let Some(stored) = self.resources.function(program) else {
            return Err(RenderError::UnknownFunction { program });
        };
        // Before the frame in every sense that matters: the pairing is refused at the
        // first command that names it, before any quad is placed and before any pipeline
        // is asked for.
        stored
            .analysis
            .admits(range)
            .map_err(|reason| RenderError::FunctionRangeRefused { program, reason })?;
        self.used_functions.insert(program.0);
        let Some(inverse) = matrix.then(self.viewport.transform).invert() else {
            return Ok(None);
        };
        Ok(Some(FunctionGeometry {
            program: program.0,
            range,
            domain: domain_bounds(domain),
            background: background_rgba(background),
            inv: [
                inverse.a, inverse.b, inverse.c, inverse.d, inverse.e, inverse.f,
            ],
        }))
    }
}

impl FunctionGeometry {
    /// The op this geometry becomes once the mark's placement is known.
    pub(super) fn at(self, placement: QuadPlacement) -> FunctionOp {
        FunctionOp {
            program: self.program,
            range: self.range,
            inv: self.inv,
            domain: self.domain,
            background: self.background,
            placement,
        }
    }
}

impl FunctionOp {
    /// The program this op paints with, as the scene names it.
    pub(crate) fn program(&self) -> FunctionId {
        FunctionId(self.program)
    }

    /// Which of ADR 0010's three styles this op draws in.
    pub(crate) fn style(&self) -> DrawStyle {
        self.placement.style
    }

    /// The soft mask this op draws under, if any.
    pub(crate) fn mask(&self) -> Option<u32> {
        self.placement.mask
    }
}

/// One [`ReportKind::FunctionEmptyStackRead`] per program this frame paints with that
/// reads a value the operand stack has not got.
///
/// ADR 0053 requires that adopting the empty-stack reading is never silent, and this is
/// where "never silent" is paid: once per distinct program per frame, from a list the
/// encode already produced, with no per-fragment cost.
///
/// **The count is static.** It is how many such reads the program *contains*, over every
/// branch, not how many the frame's pixels took — see [`ReportKind::FunctionEmptyStackRead`]
/// for why the wording follows the count rather than the other way round.
pub(crate) fn empty_stack_reports(used: &[u32], resources: &ResourceStore, into: &mut Vec<Report>) {
    for &raw in used {
        // A program released between the encode and the frame is a refusal the lane takes
        // by name; a report is not the place to raise it, so a missing one contributes
        // nothing here and `function_pipelines` says so where it matters.
        let Some(stored) = resources.function(FunctionId(raw)) else {
            continue;
        };
        let reads = stored.analysis.empty_stack_pops();
        if reads == 0 {
            continue;
        }
        into.push(Report {
            kind: ReportKind::FunctionEmptyStackRead,
            detail: format!(
                "§7.10.5 program {raw} reads an empty operand stack in {reads} place(s); \
                 ISO 32000-2 defines no value there and this device supplies 0 (ADR 0053). \
                 The count is of the program's instructions, not of the pixels that took \
                 them."
            ),
        });
    }
}

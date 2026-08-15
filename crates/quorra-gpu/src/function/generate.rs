//! WGSL for one admitted program.
//!
//! One responsibility: **printing.** Every decision about the *program* was taken by
//! [`super::analyse`]; this module resolves nothing about it. That is why it takes an
//! [`Analysis`] rather than a `&[FnOp]` — a generator that re-decided a program's
//! admission would be a second answer to "is this program drawable".
//!
//! It takes one thing the analysis does not have: the [`FnRange`], which belongs to the
//! *shading* rather than to the program. That is the one thing it can refuse, through
//! [`Analysis::admits`], and the refusal is the same one a caller can ask for before a
//! frame.
//!
//! # Determinism
//!
//! `RENDER_LIBRARY.md` §4.6 wants a frame to be a function of its inputs, and a *pipeline
//! cache keyed by a hash* needs the stronger form: the same program must produce
//! byte-identical WGSL, in every process, always. Nothing here iterates a map, reads a
//! clock, formats a pointer or depends on an environment variable; the steps are a `Vec`
//! walked in order and every float is printed by the same round-tripping formatter.
//! `tests/function_generate.rs` asserts it rather than trusting this paragraph.
//!
//! # What the emitted function is, and what it deliberately is not
//!
//! It is one free function taking the point, the domain rectangle, the range bounds and the
//! background colour as *parameters*:
//!
//! ```wgsl
//! fn quorra_function_evaluate(
//!     x: f32, y: f32,
//!     domain: vec4<f32>, range_low: vec3<f32>, range_high: vec3<f32>,
//!     background: vec4<f32>,
//! ) -> vec4<f32>
//! ```
//!
//! `x` and `y` are already in the *shading's own space* — mapping the device point through
//! the inverse `Matrix` is the composing lane's job, and doing it here would bake a
//! placement into a shader the hash says is the program's.
//!
//! No binding, no uniform block, no entry point, and none of the four numbers baked in.
//! Where they come from and which pass calls this is the lane's business; baking any of them
//! into the text would compile two pipelines for two placements of one shading.
//!
//! # The alpha channel, and why the return is a `vec4`
//!
//! ISO 32000-2 §8.7.4.5.2 governs what a type 1 shading paints where its function is not
//! defined, and it is not a clamp:
//!
//! > Points within the shading's bounding box (BBox) that fall outside this transformed
//! > domain rectangle shall be painted with the shading's background colour (Background); if
//! > the shading dictionary has no Background entry, such points shall be left unpainted.
//!
//! So the emitted function returns a colour *and a coverage*: `1.0` inside the domain, and
//! `background` unchanged outside it. An absent `Background` is `vec4<f32>(0.0)`, so "paint
//! the background" and "paint nothing" are one code path rather than two — and the shader
//! has no branch on a fact the paint already decided.
//!
//! A `Range` of [`FnRange::Gray`] still returns three channels: §8.7.4.5.2's function
//! supplies *colour components*, and one component in `DeviceGray` is the grey level, so it is
//! replicated. Returning a differently-shaped value per range would put a second signature
//! into a lane that has to call exactly one.

use std::fmt::Write as _;

use quorra_scene::FnRange;

use super::analyse::Analysis;
use super::hash::ProgramHash;
use super::lowered::{Source, Step};
use super::{ENTRY_POINT, OPERATORS};

/// The WGSL for one program, and the identity a pipeline cache keys it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedShader {
    module: String,
    /// Where [`GeneratedShader::function`] starts inside `module`, so that the two views
    /// share one allocation and cannot disagree.
    function_at: usize,
    hash: ProgramHash,
}

impl GeneratedShader {
    /// The complete WGSL module: ISO 32000-2 Table 42's operator library followed by the
    /// generated entry function. This is what a shader module is created from.
    #[must_use]
    pub fn module(&self) -> &str {
        &self.module
    }

    /// Just the generated function, without the operator library that never varies. The
    /// form a golden test should pin, and the form a person should read.
    #[must_use]
    pub fn function(&self) -> &str {
        self.module.get(self.function_at..).unwrap_or("")
    }

    /// The identity of this shader: equal exactly when the same WGSL would be emitted.
    #[must_use]
    pub fn hash(&self) -> ProgramHash {
        self.hash
    }
}

/// Emit the WGSL for an admitted program under one `Range`.
///
/// # Errors
///
/// Whatever [`Analysis::admits`] says about the `Range`: a program that cannot leave as many
/// values as the `Range` declares components, or bounds a `clamp` cannot be written against.
/// The *program* was already admitted, so nothing about it can fail here.
pub fn generate(
    analysis: &Analysis,
    range: FnRange,
) -> Result<GeneratedShader, super::FunctionRefusal> {
    analysis.admits(range)?;

    let mut emitter = Emitter {
        out: String::new(),
        indent: 1,
    };
    emitter.preamble(analysis);
    emitter.domain_test();
    emitter.slots(analysis.max_depth());
    emitter.inputs();
    emitter.steps(analysis.steps());
    emitter.outputs(analysis, range);
    emitter.out.push_str("}\n");

    let mut module = String::with_capacity(
        OPERATORS
            .len()
            .saturating_add(emitter.out.len())
            .saturating_add(1),
    );
    module.push_str(OPERATORS);
    module.push('\n');
    let function_at = module.len();
    module.push_str(&emitter.out);

    Ok(GeneratedShader {
        module,
        function_at,
        hash: analysis.shader_hash(range),
    })
}

/// A WGSL source under construction, with the current nesting.
struct Emitter {
    out: String,
    indent: usize,
}

impl Emitter {
    fn line(&mut self, text: &str) {
        for _ in 0..self.indent {
            self.out.push_str("    ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn preamble(&mut self, analysis: &Analysis) {
        let _ = write!(
            self.out,
            "// Generated from a {}-instruction ISO 32000-2 §7.10.5 type 4 function.\n\
             // Do not edit: `quorra_gpu::function::generate` emits this, and it is a pure\n\
             // function of the program, so an edit here is lost on the next frame.\n\
             //\n\
             // Operand stack: {} slot(s), reached by a walk that computed the depth rather\n\
             // than trusting one. Every `copy`/`index`/`roll` count and both readings of\n\
             // Table 42's `not` are already resolved, so nothing below is dynamic.\n\
             //\n\
             // `x` and `y` are in the shading's own space; `domain` is\n\
             // (min x, max x, min y, max y). The result's `.a` is coverage: 1 inside the\n\
             // domain rectangle, and whatever `background` carries outside it.\n\
             fn {ENTRY_POINT}(\n    \
             x: f32,\n    y: f32,\n    \
             domain: vec4<f32>,\n    range_low: vec3<f32>,\n    range_high: vec3<f32>,\n    \
             background: vec4<f32>,\n\
             ) -> vec4<f32> {{\n",
            analysis.program_length(),
            analysis.max_depth(),
        );
    }

    /// ISO 32000-2 §8.7.4.5.2's rule for a point the function does not cover.
    ///
    /// The clause, verbatim, is in this module's comment; the two things this code says that
    /// the clause does not are why it is written the way it is:
    ///
    /// - **The test is inverted.** `!(inside)` rather than `outside` puts a NaN coordinate —
    ///   which a degenerate `Matrix` can produce however finite its coefficients are — on the
    ///   *unpainted* side. Every NaN comparison is false, so the direct spelling would have
    ///   called it inside and run the program on it.
    /// - **There is no branch on whether a `Background` exists.** An absent one arrives as
    ///   `vec4<f32>(0.0)`, so "paint the background" and "paint nothing" are the same
    ///   instruction and the shader does not carry a fact the paint already settled.
    fn domain_test(&mut self) {
        self.line("// ISO 32000-2 §8.7.4.5.2: points outside the transformed domain rectangle");
        self.line("// \"shall be painted with the shading's background colour (Background); if");
        self.line("// the shading dictionary has no Background entry, such points shall be left");
        self.line("// unpainted\". Discarded, never clamped: clamping would smear the edge");
        self.line("// colour at full alpha over everything between the domain and the clip.");
        self.line("if (!(x >= domain.x && x <= domain.y && y >= domain.z && y <= domain.w)) {");
        self.line("    return background;");
        self.line("}");
    }

    /// One `var` per operand-stack slot. Initialised because WGSL requires it and because a
    /// slot an `ifelse` writes on only one arm is read on the other.
    fn slots(&mut self, max_depth: u32) {
        for slot in 0..max_depth {
            self.line(&format!("var s{slot}: f32 = 0.0;"));
        }
    }

    /// The two inputs §8.7.4.5.2 hands the function, into the slots the walk reserved.
    ///
    /// Unclamped, and that is the correction: §7.10.1's input clip is about *a function*
    /// whose argument may be out of domain, and a type 1 shading never asks its function
    /// about such a point at all — `domain_test` has already returned by then.
    fn inputs(&mut self) {
        self.line("// The point, in the shading's own space, into the two slots the walk");
        self.line("// reserved for it. Already known to be inside the domain rectangle.");
        self.line("s0 = x;");
        self.line("s1 = y;");
    }

    fn steps(&mut self, steps: &[Step]) {
        for step in steps {
            self.step(step);
        }
    }

    fn step(&mut self, step: &Step) {
        match step {
            Step::Literal { slot, value } => {
                self.line(&format!("s{slot} = {};", literal(*value)));
            }
            Step::Unary { slot, op, operand } => {
                self.line(&format!(
                    "s{slot} = {}({}); // ISO 32000-2 Table 42: {}",
                    op.wgsl(),
                    read(*operand),
                    op.table_42()
                ));
            }
            Step::Binary {
                slot,
                op,
                left,
                right,
            } => {
                self.line(&format!(
                    "s{slot} = {}({}, {}); // ISO 32000-2 Table 42: {}",
                    op.wgsl(),
                    read(*left),
                    read(*right),
                    op.table_42()
                ));
            }
            Step::Permute { writes } => self.permute(writes),
            Step::Branch {
                condition,
                on_true,
                on_false,
            } => self.branch(*condition, on_true, on_false),
        }
    }

    /// A simultaneous assignment, as reads into `let`s followed by writes.
    ///
    /// The temporaries are not decoration: `Step::Permute`'s invariant is that every source
    /// is read before any target is written, and `exch` is the case that proves it —
    /// performed as two sequential moves it would leave both slots holding one value.
    fn permute(&mut self, writes: &[(u32, Source)]) {
        self.line("{ // ISO 32000-2 Table 42, stack operator: read every source, then write");
        self.indent = self.indent.saturating_add(1);
        for (index, (_, source)) in writes.iter().enumerate() {
            self.line(&format!("let t{index} = {};", read(*source)));
        }
        for (index, (slot, _)) in writes.iter().enumerate() {
            self.line(&format!("s{slot} = t{index};"));
        }
        self.indent = self.indent.saturating_sub(1);
        self.line("}");
    }

    /// §7.10.5's `if`/`ifelse`, which reached us as a `JumpUnless` and an optional `Jump`.
    ///
    /// The test is `!= 0.0` because a boolean lives on the operand stack as 1.0 or 0.0 —
    /// see `function_ops.wgsl`'s header for why that encoding is the one that makes `and`,
    /// `or` and `xor` need a single lowering.
    fn branch(&mut self, condition: Source, on_true: &[Step], on_false: &[Step]) {
        self.line(&format!(
            "if ({} != 0.0) {{ // ISO 32000-2 §7.10.5.2: if / ifelse",
            read(condition)
        ));
        self.indent = self.indent.saturating_add(1);
        self.steps(on_true);
        self.indent = self.indent.saturating_sub(1);
        if on_false.is_empty() {
            self.line("}");
            return;
        }
        self.line("} else {");
        self.indent = self.indent.saturating_add(1);
        self.steps(on_false);
        self.indent = self.indent.saturating_sub(1);
        self.line("}");
    }

    /// The colour, clipped to the `Range`, at full coverage.
    ///
    /// This *is* a clamp, and the only one: ISO 32000-2 §7.10.1's sentence has two halves and
    /// this is the second. The first half — clipping the input to the domain — belongs to a
    /// function asked about a point outside its domain, which a type 1 shading never does
    /// (see [`Emitter::domain_test`]).
    fn outputs(&mut self, analysis: &Analysis, range: FnRange) {
        self.line("// ISO 32000-2 §7.10.1: \"output values produced by the function shall be");
        self.line("// clipped to the range\".");
        let slots = analysis.output_slots(range);
        let component = |index: usize| -> String {
            slots.get(index).map_or_else(
                // Unreachable: `generate` calls `Analysis::admits` before anything is emitted,
                // and that refuses a program which cannot leave as many values as the Range
                // declares components. Spelled out rather than unwrapped because a wrong
                // colour is worse than a black one, and `0.0` is at least a colour someone
                // asked for somewhere.
                || "0.0".to_string(),
                |slot| format!("ps_clip(s{slot}, range_low[{index}], range_high[{index}])"),
            )
        };
        match range {
            FnRange::Gray(_) => {
                self.line(&format!("let grey = {};", component(0)));
                self.line("// `DeviceGray`: one component, and the colour is that level in all");
                self.line("// three channels (§8.7.4.5.2 with §8.6.4.2).");
                self.line("return vec4<f32>(grey, grey, grey, 1.0);");
            }
            FnRange::Rgb(_) => {
                self.line(&format!(
                    "return vec4<f32>({}, {}, {}, 1.0);",
                    component(0),
                    component(1),
                    component(2)
                ));
            }
        }
    }
}

/// Where a step reads a value from.
fn read(source: Source) -> String {
    match source {
        Source::Slot(slot) => format!("s{slot}"),
        // Named rather than spelled `0.0`, so the generated shader says which decision it is
        // resting on rather than looking like arithmetic that happens to start at zero.
        Source::EmptyStackZero => "PS_EMPTY_STACK".to_string(),
    }
}

/// A literal as a WGSL `f32` literal.
///
/// `{:?}` is Rust's shortest round-tripping form, so the printed text names exactly the
/// `f32` the caller compiled — no precision is invented and none is lost. It always contains
/// a `.` or an `e`, which is what makes the result a float literal rather than an integer
/// one, and the `f` suffix pins the type so that no abstract-numeric promotion can widen it.
/// `analyse` refused every non-finite value before this runs, so `inf` and `NaN` cannot
/// reach here.
fn literal(value: f32) -> String {
    format!("{value:?}f")
}

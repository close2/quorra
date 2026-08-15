//! A paint the device evaluates: ISO 32000-2 §7.10.5 type 4 functions, admitted and lowered
//! to WGSL.
//!
//! `doc/adr/0053` is the decision this implements and `doc/spike-function-paint.md` the
//! measurement under it. In one sentence: a §8.7.4.5.2 type 1 shading whose function is a
//! PostScript calculator program reaches the caller's device today as a **grid of sampled
//! pixels**, costing them 1 142.8 ms of scene building and a four-megabyte upload per zoom
//! step for one page; the program itself is 3.9 kB, and a shader generated from it draws the
//! same page in 62 microseconds. This module is the half of that which needs no device.
//!
//! # The two units, and the seam between them
//!
//! - [`analyse`] walks the flat instruction list once and produces either an [`Analysis`] —
//!   depth, slot types, resolved counts, the agreement classification — or a
//!   [`FunctionRefusal`] naming what failed.
//! - [`generate`] turns an [`Analysis`] plus the shading's [`FnRange`] into WGSL and a
//!   [`ProgramHash`]. It resolves nothing about the program, and the only thing it can
//!   refuse is a `Range` the program cannot fill.
//!
//! The seam between them is where a program stops being one and starts being a *paint*.
//! [`analyse`] is what `Device::upload_function` runs, so a program is refused by name before
//! any frame is built; [`generate`] runs when a shading names that program with a `Range`,
//! and two shadings may name one program under different matrices and different ranges.
//!
//! Both are pure, and neither mentions `wgpu`. They live in this crate rather than in
//! `quorra-scene` because what they answer is a question about a *shader* — how many `var`s,
//! which WGSL built-in, which pipeline key — and ADR 0001 keeps the scene crate ignorant of
//! all three.
//!
//! # What is not here
//!
//! The lane. Nothing in `encode.rs`, `compose.rs` or the pipeline store knows this module
//! exists, and the paint variant is not wired to a pass. That is deliberate and is stated
//! rather than left to be discovered: a half-wired lane that draws something is exactly the
//! plausible-looking wrong page CLAUDE.md principle 6 exists to forbid. What wave 2 needs
//! from here is [`Analysis::program_hash`] to key the upload table by,
//! [`GeneratedShader::hash`] to key the pipeline by,
//! [`GeneratedShader::module`] to build it from, and
//! [`Analysis::relies_on_empty_stack`] plus [`Analysis::agreement`] to raise the two
//! `Report`s ADR 0053 requires.
//!
//! # The budgets, all three discoverable before the frame
//!
//! §5 of the brief: "if a limit must exist, it is discoverable before the frame". ISO 32000-2
//! states none of these — §7.10.5 bounds neither a program's length nor its operand stack —
//! so all three are ours, and each is a public constant a caller can compare a program
//! against without asking a device anything.

mod analyse;
mod generate;
mod hash;
mod lowered;
mod operators;
mod refusal;
mod stackops;
mod typing;
mod walk;

pub use analyse::{Analysis, INPUTS, analyse, validate_jumps};
pub use generate::{GeneratedShader, generate};
pub use hash::{GENERATOR_REVISION, ProgramHash};
pub use lowered::{Agreement, SlotType, Source, Step};
pub use operators::{Binary, Unary};
pub use refusal::FunctionRefusal;

use quorra_scene::{Color, FnRange, Rect};

/// ISO 32000-2 Table 42, one WGSL function per operator, plus §7.10.1's output clip.
///
/// Public because it is half of what [`GeneratedShader::module`] returns and a reader of a
/// generated shader needs the other half to make sense of it. It is a constant, it never
/// varies with a program, and it is hashed into every [`ProgramHash`] so that an edit to it
/// invalidates the pipelines built before the edit.
pub const OPERATORS: &str = include_str!("shaders/function_ops.wgsl");

/// The name of the function [`generate`] emits.
///
/// One name for every program: the shader module differs per program and is compiled
/// separately, so nothing is gained by making the symbol differ too — and a stable name is
/// what lets a composing pass be written against the signature rather than against a hash.
pub const ENTRY_POINT: &str = "quorra_function_evaluate";

/// The longest program admitted, in instructions.
///
/// The generated shader's length is linear in the program's, and its *compile* is what sits
/// on the caller's first-frame path — measured at 6.3 ms cold for a 482-instruction witness
/// (`doc/spike-function-paint.md` §3). This bound keeps the worst case around a megabyte of
/// WGSL rather than around a gigabyte; ISO 32000-2 states no limit, so it is ours, and the
/// two witnesses that exist need 482 and 311.
pub const MAX_PROGRAM_LENGTH: usize = 8192;

/// The deepest operand stack admitted, in slots.
///
/// One `var` in the generated shader per slot, so this is what bounds a shader's register
/// pressure. Both of the caller's witnesses reach 8; the spike's own array was 64 and never
/// came near it. ISO 32000-2 states no limit and PLRM3's Appendix B calls its own table
/// "typical" rather than required, so this is ours.
pub const MAX_OPERAND_SLOTS: usize = 64;

/// The deepest `if`/`ifelse` nesting admitted.
///
/// The walk recurses once per level, so this bounds a stack that document-derived data
/// would otherwise choose the depth of — CLAUDE.md principle 3's "never allocate from an
/// unchecked number", applied to the one allocation a recursive descent makes without
/// asking. The seven-segment witness has 23 conditionals and nests 3 deep.
pub const MAX_BRANCH_NESTING: usize = 64;

/// The `Range`'s bounds as the two `vec3<f32>` the generated function takes.
///
/// [`FnRange::Gray`] replicates its one pair into all three lanes, so that a caller of
/// [`ENTRY_POINT`] passes the same shape whichever range it holds — the alternative is a
/// branch at every call site over a fact that was already decided when the paint was built.
#[must_use]
pub fn range_bounds(range: FnRange) -> ([f32; 3], [f32; 3]) {
    match range {
        FnRange::Gray([low, high]) => ([low; 3], [high; 3]),
        FnRange::Rgb(components) => (
            [components[0][0], components[1][0], components[2][0]],
            [components[0][1], components[1][1], components[2][1]],
        ),
    }
}

/// The `Domain` rectangle as the `vec4<f32>` the generated function takes:
/// `(min x, max x, min y, max y)`.
///
/// Not `(min x, min y, max x, max y)`, so that the emitted test reads
/// `x >= domain.x && x <= domain.y` — the two bounds of one axis side by side, which is how
/// a reader checks it.
///
/// The generated shader assumes the rectangle is ordered and finite; `Rect::is_ordered` and
/// `Rect::is_finite` are that check and belong at the scene boundary with every other
/// rectangle's.
#[must_use]
pub fn domain_bounds(domain: Rect) -> [f32; 4] {
    [domain.min.x, domain.max.x, domain.min.y, domain.max.y]
}

/// §8.7.4.5.2's `Background` as the `vec4<f32>` the generated function takes.
///
/// **`None` is `vec4<f32>(0.0)`, and that is the whole of the encoding.** The clause gives
/// two outcomes outside the domain rectangle — paint the background, or leave the point
/// unpainted — and a colour with zero alpha *is* the second, so the shader needs no branch
/// on which case a paint is in.
#[must_use]
pub fn background_rgba(background: Option<Color>) -> [f32; 4] {
    match background {
        Some(colour) => [colour.r, colour.g, colour.b, colour.a],
        None => [0.0; 4],
    }
}

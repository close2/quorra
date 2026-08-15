#![allow(
    clippy::float_cmp,
    clippy::cast_precision_loss,
    reason = "a boolean on the operand stack is exactly 0.0 or 1.0, so the branch test is an \
              exact comparison by construction; and the integer-to-float conversion is the \
              one the operand stack performs, whose loss past 2^24 is the representation \
              rather than the code"
)]

//! The *lowered* program, interpreted over a slot array rather than a stack. **Tests only.**
//!
//! One responsibility, and it is deliberately not [`super::reference`]'''s: this runs
//! `Analysis::steps()`, so comparing the two checks **the lowering** — slot allocation, the
//! reads-before-writes invariant of `Step::Permute`, the branch structure recovered from the
//! jumps, and the resolution of Table 42'''s two `not`s.
//!
//! The arithmetic is deliberately shared with [`super::reference`], because arithmetic is
//! what `tests/function_device.rs` checks against the shader. Two tests, two questions, and
//! neither one alone would find both kinds of defect.

use quorra_gpu::function::{Analysis, Binary, Source, Step, Unary};
use quorra_scene::FnOp;

use super::programs::Witness;
use super::reference::{Value, apply, clip_outputs, pop, to_int};

/// Interpret the *lowered* form the analyser produced, over a slot array rather than a stack.
///
/// Compared against [`evaluate_shading`], this tests the lowering and nothing else: slot
/// allocation, the reads-before-writes invariant of `Step::Permute`, the branch structure
/// recovered from the jumps, and the resolution of Table 42's two `not`s. The arithmetic is
/// deliberately shared with [`evaluate`], because arithmetic is what the *device* test checks.
#[must_use]
pub fn run_lowered(analysis: &Analysis, witness: &Witness, x: f32, y: f32) -> [f32; 4] {
    let inside = x >= witness.domain.min.x
        && x <= witness.domain.max.x
        && y >= witness.domain.min.y
        && y <= witness.domain.max.y;
    if !inside {
        return match witness.background {
            Some(colour) => [colour.r, colour.g, colour.b, colour.a],
            None => [0.0; 4],
        };
    }

    let mut slots = vec![0.0_f32; analysis.max_depth() as usize];
    if let Some(slot) = slots.get_mut(0) {
        *slot = x;
    }
    if let Some(slot) = slots.get_mut(1) {
        *slot = y;
    }
    run_steps(analysis.steps(), &mut slots);

    // The slots the outputs live in, derived here rather than asked for, because the analysis
    // deliberately does not expose them: it knows nothing about a `Range`.
    let depth = analysis.values_left() as usize;
    let components = witness.range.components();
    let base = depth.saturating_sub(components);
    let stack: Vec<f32> = slots.get(base..depth).unwrap_or(&[]).to_vec();
    let [red, green, blue] = clip_outputs(&stack, witness.range);
    [red, green, blue, 1.0]
}

fn read_source(source: Source, slots: &[f32]) -> f32 {
    match source {
        Source::Slot(slot) => slots.get(slot as usize).copied().unwrap_or(0.0),
        Source::EmptyStackZero => 0.0,
    }
}

fn write_slot(slots: &mut [f32], slot: u32, value: f32) {
    if let Some(cell) = slots.get_mut(slot as usize) {
        *cell = value;
    }
}

fn run_steps(steps: &[Step], slots: &mut Vec<f32>) {
    for step in steps {
        match step {
            Step::Literal { slot, value } => write_slot(slots, *slot, *value),
            Step::Unary { slot, op, operand } => {
                let value = apply_unary(*op, read_source(*operand, slots));
                write_slot(slots, *slot, value);
            }
            Step::Binary {
                slot,
                op,
                left,
                right,
            } => {
                let mut stack = vec![
                    Value::real(read_source(*left, slots)),
                    Value::real(read_source(*right, slots)),
                ];
                apply(binary_op(*op), &mut stack);
                write_slot(slots, *slot, pop(&mut stack).number);
            }
            Step::Permute { writes } => {
                // The invariant, honoured the way the generated WGSL honours it.
                let read: Vec<f32> = writes
                    .iter()
                    .map(|(_, source)| read_source(*source, slots))
                    .collect();
                for ((slot, _), value) in writes.iter().zip(read) {
                    write_slot(slots, *slot, value);
                }
            }
            Step::Branch {
                condition,
                on_true,
                on_false,
            } => {
                if read_source(*condition, slots) == 0.0 {
                    run_steps(on_false, slots);
                } else {
                    run_steps(on_true, slots);
                }
            }
        }
    }
}

/// The lowered form has already chosen which `not` an occurrence meant, so this needs no type
/// at all — which is the property `function::analyse` exists to buy.
fn apply_unary(op: Unary, value: f32) -> f32 {
    match op {
        Unary::LogicalNot => return f32::from(value == 0.0),
        Unary::BitwiseNot => return !to_int(value) as f32,
        _ => {}
    }
    let mut stack = vec![Value::real(value)];
    apply(unary_op(op), &mut stack);
    pop(&mut stack).number
}

fn unary_op(op: Unary) -> FnOp {
    match op {
        Unary::Abs => FnOp::Abs,
        Unary::Ceiling => FnOp::Ceiling,
        Unary::Cos => FnOp::Cos,
        Unary::Cvi => FnOp::Cvi,
        Unary::Cvr => FnOp::Cvr,
        Unary::Floor => FnOp::Floor,
        Unary::Ln => FnOp::Ln,
        Unary::Log => FnOp::Log,
        Unary::Neg => FnOp::Neg,
        Unary::Round => FnOp::Round,
        Unary::Sin => FnOp::Sin,
        Unary::Sqrt => FnOp::Sqrt,
        Unary::Truncate => FnOp::Truncate,
        Unary::LogicalNot | Unary::BitwiseNot => FnOp::Not,
    }
}

fn binary_op(op: Binary) -> FnOp {
    match op {
        Binary::Add => FnOp::Add,
        Binary::Atan => FnOp::Atan,
        Binary::Div => FnOp::Div,
        Binary::Exp => FnOp::Exp,
        Binary::Idiv => FnOp::Idiv,
        Binary::Mod => FnOp::Mod,
        Binary::Mul => FnOp::Mul,
        Binary::Sub => FnOp::Sub,
        Binary::And => FnOp::And,
        Binary::Bitshift => FnOp::Bitshift,
        Binary::Eq => FnOp::Eq,
        Binary::Ge => FnOp::Ge,
        Binary::Gt => FnOp::Gt,
        Binary::Le => FnOp::Le,
        Binary::Lt => FnOp::Lt,
        Binary::Ne => FnOp::Ne,
        Binary::Or => FnOp::Or,
        Binary::Xor => FnOp::Xor,
    }
}

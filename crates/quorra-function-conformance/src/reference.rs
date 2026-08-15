//! The reference evaluator: one reading of ISO 32000-2 §7.10.5, written to be argued
//! with.
//!
//! This is the oracle the device is compared against, so where it came from matters as
//! much as what it does. It was written from ISO 32000-2 §7.10 and §7.10.5, Annex B's
//! operator summary, and the PLRM3 entries §7.10.5.2 makes normative — **not** from the
//! spike's `eval.rs`, not from the caller's evaluator, and not from any generator in
//! this tree. Two independent implementations of one clause disagreeing is a finding;
//! one implementation checked against a copy of itself is not.
//!
//! # What an evaluation is
//!
//! §7.10.5.3 states the shape:
//!
//! > The input variables shall constitute the initial operand stack; the items remaining
//! > on the operand stack after execution of the function shall be the output variables.
//! > It shall be an error for the number of remaining operands to differ from the number
//! > of output variables specified by **Range** or for any of them to be objects other
//! > than numbers.
//!
//! and §7.10.1 wraps two clips around it:
//!
//! > Input values passed to the function shall be clipped to the domain, and output
//! > values produced by the function shall be clipped to the range.
//!
//! So an evaluation is: clip the inputs, push them, run to the end, check the count,
//! clip the outputs. Every one of those five steps is normative and two of them are
//! invisible on both of the caller's witnesses, which declare the unit interval at both
//! ends.
//!
//! # What it refuses to do
//!
//! It never produces a number the two documents do not define. A division by zero, a
//! square root of a negative, a `bitshift` past the operand's width and a non-finite
//! output are each returned as an [`EvalError`] naming which of the three kinds of
//! not-a-number it is. That is principle 6 applied to an oracle: a plausible value here
//! would become a plausible expectation in the corpus, and then a device that produced
//! it would pass.

pub mod arithmetic;
pub mod error;
pub mod relational;
pub mod stack;
pub mod stack_ops;
pub mod value;

use crate::case::{Case, PsError, Report};
use error::EvalError;
use quorra_scene::function::{FnOp, FnRange};
use stack::Stack;
use value::Value;

/// What a program left behind, and what it relied on to get there.
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    /// One value per output component, clipped into `Range`.
    pub outputs: Vec<f32>,
    /// Choices made where the specification defines nothing, which a frame must carry
    /// rather than adopt silently.
    pub reports: Vec<Report>,
}

/// Evaluate a case exactly as its own `Domain`, `Range` and inputs describe it.
///
/// # Errors
///
/// As [`evaluate`].
pub fn evaluate_case(case: &Case) -> Result<Evaluation, EvalError> {
    evaluate(case.program, case.inputs, case.domain, case.range)
}

/// Evaluate a compiled §7.10.5 program.
///
/// # Errors
///
/// [`EvalError::Error`] where an operator's PLRM3 entry names one, [`EvalError::Undefined`]
/// where neither document defines a result, and [`EvalError::Malformed`] where the
/// instruction list is not a well-formed compiled program.
pub fn evaluate(
    program: &[FnOp],
    inputs: &[f32],
    domain: [f32; 4],
    range: FnRange,
) -> Result<Evaluation, EvalError> {
    let mut stack = Stack::new();
    push_clipped_inputs(&mut stack, inputs, domain)?;

    let mut pc = 0usize;
    while pc < program.len() {
        pc = step(program[pc], &mut stack, pc, program.len())?;
    }

    Ok(Evaluation {
        outputs: clipped_outputs(&stack, range)?,
        reports: stack.reports().to_vec(),
    })
}

/// §7.10.1's first clip, and Table 38's `Domain` row: "Input values outside the declared
/// domain shall be clipped to the nearest boundary value."
fn push_clipped_inputs(
    stack: &mut Stack,
    inputs: &[f32],
    domain: [f32; 4],
) -> Result<(), EvalError> {
    if inputs.len() > 2 {
        return Err(EvalError::Malformed(
            "more inputs than the four-number Domain of a §8.7.4.5.2 shading describes",
        ));
    }
    // One `[min, max]` pair per input, in order, which is how Table 38 writes the row.
    for (bounds, value) in domain.chunks_exact(2).zip(inputs) {
        stack.push(Value::Real(value.max(bounds[0]).min(bounds[1])))?;
    }
    Ok(())
}

/// One instruction, returning the next program counter.
fn step(op: FnOp, stack: &mut Stack, pc: usize, length: usize) -> Result<usize, EvalError> {
    let next = pc.saturating_add(1);
    match op {
        FnOp::PushReal(value) => stack.push(Value::Real(value)).map(|()| next),
        FnOp::PushInt(value) => stack.push(Value::Int(value)).map(|()| next),
        FnOp::PushBool(value) => stack.push(Value::Bool(value)).map(|()| next),
        FnOp::Jump { target } => jump_target(target, pc, length),
        FnOp::JumpUnless { target } => {
            // PLRM3's `if`: "removes both operands from the stack, then executes proc if
            // bool is true". The condition is a boolean and nothing else.
            if stack.pop().boolean()? {
                Ok(next)
            } else {
                jump_target(target, pc, length)
            }
        }
        _ => apply(op, stack).map(|()| next),
    }
}

/// A jump is forward or the program is not one of ours: that is what makes the
/// instruction count an execution bound, and it is checked rather than assumed.
fn jump_target(target: u32, pc: usize, length: usize) -> Result<usize, EvalError> {
    let target = usize::try_from(target)
        .map_err(|_| EvalError::Malformed("a jump target that does not fit an index"))?;
    if target <= pc {
        return Err(EvalError::Malformed(
            "a jump that is not strictly forward, which would remove the execution bound",
        ));
    }
    if target > length {
        return Err(EvalError::Malformed(
            "a jump target past the end of the program",
        ));
    }
    Ok(target)
}

/// Dispatch to the family that owns the operator.
fn apply(op: FnOp, stack: &mut Stack) -> Result<(), EvalError> {
    if let Some(arity) = arithmetic::arity(op) {
        let value = match arity {
            arithmetic::Arity::Unary => {
                let a = stack.pop();
                arithmetic::unary(op, a)?
            }
            arithmetic::Arity::Binary => {
                let (b, a) = (stack.pop(), stack.pop());
                arithmetic::binary(op, a, b)?
            }
        };
        return stack.push(value);
    }
    if relational::is_binary(op) {
        let (b, a) = (stack.pop(), stack.pop());
        let value = relational::binary(op, a, b)?;
        return stack.push(value);
    }
    if matches!(op, FnOp::Not) {
        let a = stack.pop();
        let value = relational::not(a)?;
        return stack.push(value);
    }
    if stack_ops::is_stack_operator(op) {
        return stack_ops::apply(op, stack);
    }
    Err(EvalError::Malformed(
        "an instruction that is neither a literal, a jump, nor a Table 42 operator",
    ))
}

/// §7.10.5.3's count-and-type rule, then §7.10.1's second clip.
fn clipped_outputs(stack: &Stack, range: FnRange) -> Result<Vec<f32>, EvalError> {
    let values = stack.values();
    if values.len() != range.components() || !values.iter().all(|value| value.is_number()) {
        return Err(PsError::OutputCount.into());
    }
    let mut outputs = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let raw = value.real()?;
        // Checked *before* the clip on purpose. Clipping a NaN against a range with
        // `max`/`min` yields a boundary value, and clipping an infinity yields the
        // boundary itself — either would turn "this program has no result" into a
        // perfectly ordinary colour, which is principle 6's worst outcome by name.
        if !raw.is_finite() {
            return Err(EvalError::Undefined(
                "a non-finite output: ISO 32000-2 §7.3.3 defers the range of a number to \
                 the machine and states no result for an overflow, and PLRM3 defers again",
            ));
        }
        let [low, high] = range
            .bounds()
            .get(index)
            .copied()
            .ok_or(EvalError::Malformed("a Range with too few components"))?;
        outputs.push(raw.max(low).min(high));
    }
    Ok(outputs)
}

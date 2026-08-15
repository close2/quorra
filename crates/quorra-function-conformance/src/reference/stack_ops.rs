//! Table 42's six stack operators, from PLRM3's entries and ISO 32000-2 Annex B's
//! stack diagrams.
//!
//! None of them computes anything, and three of them take a count *off the stack* —
//! which is why `copy`, `index` and `roll` are the operators a generated shader has to
//! resolve statically or refuse: a slot it cannot name at compile time is a slot it
//! cannot have.

use super::error::EvalError;
use super::stack::Stack;
use super::value::Value;
use crate::case::PsError;
use quorra_scene::function::FnOp;

/// Whether `op` is one of the six.
#[must_use]
pub const fn is_stack_operator(op: FnOp) -> bool {
    matches!(
        op,
        FnOp::Copy | FnOp::Dup | FnOp::Exch | FnOp::Index | FnOp::Pop | FnOp::Roll
    )
}

/// Apply a stack operator.
///
/// # Errors
///
/// `typecheck` for a count that is not an integer, `rangecheck` for a negative one or
/// for an `index` past the bottom, `stackunderflow` where `copy` or `roll` name more
/// elements than are present, and `stackoverflow` past §7.10.5.3's 100 entries.
pub fn apply(op: FnOp, stack: &mut Stack) -> Result<(), EvalError> {
    match op {
        // "removes the top element from the operand stack and discards it."
        FnOp::Pop => {
            stack.pop();
            Ok(())
        }
        // "duplicates the top element on the operand stack."
        FnOp::Dup => {
            let top = stack.pop();
            stack.push(top)?;
            stack.push(top)
        }
        // "exchanges the top two elements on the operand stack."
        FnOp::Exch => {
            let (top, below) = (stack.pop(), stack.pop());
            stack.push(top)?;
            stack.push(below)
        }
        FnOp::Copy => copy(stack),
        FnOp::Index => index(stack),
        FnOp::Roll => roll(stack),
        _ => Err(EvalError::Malformed("not a stack operator")),
    }
}

/// `copy`: "where the top element on the operand stack is a nonnegative integer n, copy
/// pops n from the stack and duplicates the top n elements on the stack".
///
/// `(a) (b) (c) 0 copy ⇒ (a) (b) (c)` is one of the entry's own examples, so a count of
/// zero is a no-op rather than a degenerate case.
fn copy(stack: &mut Stack) -> Result<(), EvalError> {
    let count = nonnegative_count(stack.pop())?;
    if count > stack.depth() {
        return Err(PsError::StackUnderflow.into());
    }
    // Read all n before pushing any: each push moves the top, so a loop that peeks and
    // pushes alternately duplicates its own output from the second element on.
    let mut duplicated = Vec::with_capacity(count);
    for position in (0..count).rev() {
        duplicated.push(stack.peek(position).ok_or(PsError::StackUnderflow)?);
    }
    for value in duplicated {
        stack.push(value)?;
    }
    Ok(())
}

/// `index`: "removes the nonnegative integer n from the operand stack, counts down to
/// the nth element from the top of the stack, and pushes a copy of that element".
fn index(stack: &mut Stack) -> Result<(), EvalError> {
    let position = nonnegative_count(stack.pop())?;
    let value = stack.peek(position).ok_or(PsError::RangeCheck)?;
    stack.push(value)
}

/// `roll`: "performs a circular shift of the objects anyn−1 through any0 on the operand
/// stack by the amount j. Positive j indicates upward motion on the stack".
///
/// "n must be a nonnegative integer and j must be an integer", and the shift itself is
/// [`Stack::rotate_top`].
fn roll(stack: &mut Stack) -> Result<(), EvalError> {
    let amount = stack.pop().integer()?;
    let count = nonnegative_count(stack.pop())?;
    stack.rotate_top(count, amount)
}

/// A count operand: an integer by `typecheck`, nonnegative by `rangecheck`.
fn nonnegative_count(value: Value) -> Result<usize, PsError> {
    usize::try_from(value.integer()?).map_err(|_| PsError::RangeCheck)
}

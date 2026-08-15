//! What a compiled program may not be, and it is refused loudly.
//!
//! This is the **structural** half of admitting a §7.10.5 program: is the instruction
//! list a thing that can be executed at all. It runs at `Device::upload_function`, not at
//! the scene boundary, and that placement is the point — a program is an uploaded
//! resource, so a caller learns its program is unsupported *before it has built a scene*,
//! which is §5's "discoverable before the frame" satisfied properly rather than by
//! accident.
//!
//! It lives here, in `quorra-scene`, because the checks are about the vocabulary this
//! crate defines and because a caller may want the answer without a device in hand. It is
//! public API for that reason.
//!
//! # What is deliberately **not** checked here
//!
//! The *semantic* half — stack depth, the static classification of the operators two
//! adapters do not agree on (ADR 0053 §3), the resolution of `copy`/`index`/`roll`
//! counts, and a [`FnOp::JumpUnless`] whose condition is not a boolean — is `quorra-gpu`'s
//! analyser. All four need a walk that models the stack and its types; this one needs
//! only the list. A program that passes here may still be refused there, and that is the
//! division rather than a gap.
//!
//! The paint that *references* a program — its domain, matrix, range and background — is
//! checked at the scene boundary instead, with the other §4.7 refusals.

use super::FnOp;
use crate::error::SceneError;

/// The most instructions an uploaded §7.10.5 program may hold.
///
/// ISO 32000-2 bounds a type 4 function's length nowhere, so this is a deliberate choice
/// of ours with its cost written down rather than a number read out of a clause.
///
/// The anchor is `doc/spike-function-paint.md`: the largest type 4 program in either of
/// the caller's two witnesses is **482 instructions**, and the generated shader for it
/// took **6.3 ms** to compile with a cold driver cache. 8 192 is seventeen times that
/// witness; extrapolating the compile cost linearly — an extrapolation, not a
/// measurement — puts a program at this bound near **107 ms** of pipeline compilation,
/// which is eighteen times the whole 5.9 ms CPU-rasteriser frame principle 2 measures
/// against. So the bound is a **ceiling that keeps a refusal cheap**, not a target: a
/// program approaching it will be refused by a device budget long before it is drawn
/// (ADR 0053 §1 — the interpreter shape lost the device outright at 482 instructions
/// times 32 million fragments, and that is a refusal that must happen before the frame).
///
/// It also bounds what one upload can be made to hold: 8 192 × 8 bytes is 64 KiB.
pub const MAX_PROGRAM_LENGTH: usize = 8_192;

/// Whether a compiled program is structurally executable, or which condition it broke.
///
/// The three conditions, in the order a reader meets them: it produces something, it is
/// within the stated bound, and every jump moves strictly forward into the list.
///
/// # Errors
///
/// - [`SceneError::EmptyFunctionProgram`] — no instructions, so no output value.
/// - [`SceneError::FunctionProgramTooLong`] — past [`MAX_PROGRAM_LENGTH`], which the
///   error names.
/// - [`SceneError::BackwardFunctionJump`] — a jump to itself or backwards.
/// - [`SceneError::FunctionJumpOutOfRange`] — a jump past the end of the list.
pub fn check_program(program: &[FnOp]) -> Result<(), SceneError> {
    if program.is_empty() {
        // §7.10.1: a function is a transformation that produces output values. An empty
        // program produces none, and the empty-stack rule (ADR 0053) would turn that into
        // a plausible black instead of a refusal.
        return Err(SceneError::EmptyFunctionProgram);
    }
    if program.len() > MAX_PROGRAM_LENGTH {
        return Err(SceneError::FunctionProgramTooLong {
            length: program.len(),
            limit: MAX_PROGRAM_LENGTH,
        });
    }
    // The length is at most MAX_PROGRAM_LENGTH by the check above, so it fits u32; the
    // conversion is written fallibly anyway, and its error is the same bound, so the
    // claim is enforced rather than asserted in a comment.
    let length = u32::try_from(program.len()).map_err(|_| SceneError::FunctionProgramTooLong {
        length: program.len(),
        limit: MAX_PROGRAM_LENGTH,
    })?;
    // Counting alongside the instructions rather than indexing them: every position a
    // jump can name is then a `u32` by construction.
    for (at, op) in (0..length).zip(program.iter()) {
        if let FnOp::Jump { target } | FnOp::JumpUnless { target } = *op {
            check_jump(at, target, length)?;
        }
    }
    Ok(())
}

/// One jump, and the two conditions in the order that makes the message useful.
///
/// Forward-only is checked first because it is the property the whole design rests on: a
/// flat list whose jumps only ever move forward cannot loop, so the program's length is a
/// bound on its own execution and a fragment shader needs no loop to run it. A backward
/// or self jump is not slow, it is unbounded, and no device budget can be stated over it.
///
/// `target == length` is legitimate and means "stop": it is how a trailing `if` lowers,
/// the true arm being the tail of the program.
fn check_jump(at: u32, target: u32, length: u32) -> Result<(), SceneError> {
    if target <= at {
        return Err(SceneError::BackwardFunctionJump { at, target });
    }
    if target > length {
        return Err(SceneError::FunctionJumpOutOfRange { at, target, length });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MAX_PROGRAM_LENGTH, check_program};
    use crate::error::SceneError;
    use crate::function::FnOp;

    /// The positive case, stated first so that every refusal below differs from it in
    /// exactly one way: the shape an `ifelse` lowers to, with both jumps forward.
    #[test]
    fn a_program_whose_jumps_go_forward_is_accepted() {
        let program = [
            FnOp::PushBool(true),
            FnOp::JumpUnless { target: 4 },
            FnOp::PushReal(1.0),
            FnOp::Jump { target: 5 },
            FnOp::PushReal(0.0),
            FnOp::Dup,
            FnOp::Dup,
        ];
        check_program(&program).expect("forward jumps are the whole contract");
    }

    /// An empty program leaves no output value. The empty-stack rule would turn that into
    /// a plausible black, which is the outcome §5 refuses by name.
    #[test]
    fn an_empty_program_is_refused_at_the_boundary() {
        assert!(matches!(
            check_program(&[]),
            Err(SceneError::EmptyFunctionProgram)
        ));
    }

    /// The bound exists so that a refusal is cheap and an upload's memory is bounded; it
    /// names itself, per §5's "an `Err` that names what overflowed".
    #[test]
    fn a_program_past_the_stated_bound_is_refused_with_its_limit() {
        let too_long = vec![FnOp::Pop; MAX_PROGRAM_LENGTH + 1];
        assert!(matches!(
            check_program(&too_long),
            Err(SceneError::FunctionProgramTooLong {
                length,
                limit: MAX_PROGRAM_LENGTH,
            }) if length == MAX_PROGRAM_LENGTH + 1
        ));
        let at_the_bound = vec![FnOp::Pop; MAX_PROGRAM_LENGTH];
        check_program(&at_the_bound).expect("the bound itself is admitted");
    }

    /// The property the whole design rests on: a jump that does not move strictly forward
    /// makes the program's length no bound on its own execution.
    #[test]
    fn a_backward_jump_is_refused_at_the_boundary() {
        let jump_back = FnOp::Jump { target: 0 };
        let program = [FnOp::PushReal(0.0), FnOp::PushReal(0.0), jump_back];
        assert!(matches!(
            check_program(&program),
            Err(SceneError::BackwardFunctionJump { at: 2, target: 0 })
        ));
    }

    /// A jump to itself is the same defect with a zero-length loop, and is refused by the
    /// same condition rather than falling through it.
    #[test]
    fn a_self_jump_is_refused_at_the_boundary() {
        let program = [FnOp::PushBool(true), FnOp::JumpUnless { target: 1 }];
        assert!(matches!(
            check_program(&program),
            Err(SceneError::BackwardFunctionJump { at: 1, target: 1 })
        ));
    }

    /// Past the end is a different defect from backwards, and gets its own name and the
    /// length it exceeded.
    #[test]
    fn a_jump_past_the_end_of_the_program_is_refused() {
        let program = [FnOp::PushBool(true), FnOp::JumpUnless { target: 9 }];
        assert!(matches!(
            check_program(&program),
            Err(SceneError::FunctionJumpOutOfRange {
                at: 1,
                target: 9,
                length: 2
            })
        ));
    }

    /// A jump *to* the end is how a trailing `if` lowers — the true arm is the tail — so
    /// it is accepted, and it means "stop".
    #[test]
    fn a_jump_to_the_end_of_the_program_terminates_it() {
        let program = [
            FnOp::PushBool(true),
            FnOp::JumpUnless { target: 3 },
            FnOp::PushReal(1.0),
        ];
        check_program(&program)
            .expect("falling off the end is how a trailing conditional finishes");
    }
}

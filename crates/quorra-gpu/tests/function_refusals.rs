#![allow(
    clippy::expect_used,
    reason = "a refusal helper whose panic is the test failing"
)]

//! Every refusal ground, on a program that reaches it.
//!
//! `doc/spike-function-paint.md` §6 states the rule this file exists to keep: **a ground
//! nobody can reach is not a ground.** So there is exactly one test per variant of
//! `FunctionRefusal`, each named for the program's defect rather than for the variant, and the
//! file is expected to grow a test whenever the enum grows a variant.
//!
//! Three of the spike's grounds are deliberately absent and *cannot* be tested, which is the
//! stronger result: "an operator outside Table 42", "unbalanced braces" and "a stray
//! procedure" were grounds because the spike compiled PostScript text. `FnOp` is a closed enum
//! with no procedure in it, so none of the three is expressible here — the caller's compiler
//! owns them.
//!
//! The two `Range` grounds are in `function_analysis.rs` instead, with the rest of
//! `Analysis::admits`: a `Range` belongs to the shading rather than to the program, and so
//! does the moment it is refused.

mod function_support;

use function_support::programs;
use quorra_gpu::function::{
    FunctionRefusal, MAX_BRANCH_NESTING, MAX_OPERAND_SLOTS, MAX_PROGRAM_LENGTH, analyse,
};
use quorra_scene::FnOp;

fn refuse(program: &[FnOp]) -> FunctionRefusal {
    analyse(program).expect_err("this program should have been refused")
}

/// The compiled form is a closed enum, so the spike's first ground is not expressible. Kept as
/// a test because "we removed a refusal" is a claim, and this is the check on it.
#[test]
fn a_closed_operator_set_needs_no_refusal() {
    // Every `FnOp` is a Table 42 operator or one of the two the caller compiles `if`/`ifelse`
    // to. If that ever stops being true, the line below stops compiling, which is the notice.
    let every_kind: &[FnOp] = &[FnOp::Abs, FnOp::And, FnOp::Copy, FnOp::Jump { target: 1 }];
    assert_eq!(every_kind.len(), 4);
}

/// The generated shader's length is linear in the program's, and its compile sits on the
/// caller's first-frame path.
#[test]
fn a_program_past_the_length_budget_is_refused() {
    let program = vec![FnOp::Dup; MAX_PROGRAM_LENGTH + 1];
    assert_eq!(
        refuse(&program),
        FunctionRefusal::ProgramTooLong {
            length: MAX_PROGRAM_LENGTH + 1,
            limit: MAX_PROGRAM_LENGTH,
        }
    );
}

/// Forward-only jumping is what makes a program's length a bound on its own execution, so it
/// is proved rather than assumed.
#[test]
fn a_backward_jump_is_refused() {
    let program = &[FnOp::Pop, FnOp::Pop, FnOp::Jump { target: 0 }];
    assert_eq!(
        refuse(program),
        FunctionRefusal::BackwardJump { at: 2, target: 0 }
    );
}

/// A jump past the end has no instruction to resume at, and a shader has nowhere to put it.
#[test]
fn a_jump_past_the_end_is_refused() {
    let program = &[FnOp::PushBool(true), FnOp::JumpUnless { target: 9 }];
    assert_eq!(
        refuse(program),
        FunctionRefusal::JumpOutOfRange {
            at: 1,
            target: 9,
            length: 2,
        }
    );
}

/// An unconditional jump reached in sequence is a `goto`. `if` and `ifelse` are the only
/// control flow §7.10.5.1 admits and both nest, so nothing legal produces this — and a
/// generated shader has nothing to lower it to.
#[test]
fn a_jump_that_does_not_nest_is_refused() {
    let program = &[
        FnOp::PushReal(1.0),
        FnOp::Jump { target: 3 },
        FnOp::PushReal(2.0),
        FnOp::PushReal(3.0),
    ];
    assert_eq!(
        refuse(program),
        FunctionRefusal::UnstructuredControlFlow { at: 1 }
    );
}

/// The walk recurses once per nesting level, so the depth of that recursion is a number
/// document-derived data would otherwise choose (CLAUDE.md principle 3).
#[test]
fn branches_past_the_nesting_budget_are_refused() {
    let levels = MAX_BRANCH_NESTING + 1;
    let end = u32::try_from(levels * 2).unwrap();
    let mut program = Vec::new();
    for _ in 0..levels {
        program.push(FnOp::PushBool(true));
        // Every arm ends at the last instruction, which is legal and maximally nested.
        program.push(FnOp::JumpUnless { target: end });
    }
    program.push(FnOp::PushReal(0.5));

    let refusal = refuse(&program);
    assert!(
        matches!(
            refusal,
            FunctionRefusal::BranchesTooDeep {
                limit: MAX_BRANCH_NESTING,
                ..
            }
        ),
        "{refusal}"
    );
}

/// One `var` per slot: the count has to be known and bounded before the shader exists.
#[test]
fn an_operand_stack_past_the_slot_budget_is_refused() {
    let program = vec![FnOp::Dup; MAX_OPERAND_SLOTS];
    assert_eq!(
        refuse(&program),
        FunctionRefusal::StackTooDeep {
            needed: MAX_OPERAND_SLOTS + 1,
            limit: MAX_OPERAND_SLOTS,
        }
    );
}

/// A generated shader cannot name a slot it cannot compute — the pinned decision, and the one
/// refusal that is about the *shape* being built rather than about the program.
#[test]
fn a_computed_stack_count_is_refused() {
    // `1 1 add` is a count no walk can name without folding constants, which this one
    // deliberately does not do. `roll` takes its shift off the stack *first*, so it needs one
    // more literal for the computed value to reach the count rather than the shift.
    let computed = [FnOp::PushInt(1), FnOp::PushInt(1), FnOp::Add];
    let cases: [(&'static str, Vec<FnOp>); 3] = [
        ("copy", [computed.as_slice(), &[FnOp::Copy]].concat()),
        ("index", [computed.as_slice(), &[FnOp::Index]].concat()),
        (
            "roll",
            [computed.as_slice(), &[FnOp::PushInt(1), FnOp::Roll]].concat(),
        ),
    ];
    for (operator, program) in cases {
        assert_eq!(
            refuse(&program),
            FunctionRefusal::DynamicStackCount { operator },
            "{operator}"
        );
    }
}

/// PLRM3 makes an out-of-range count a `rangecheck`. The pinned "a pop from an empty stack
/// yields 0" covers a *pop*; it does not licence naming operands the stack never had.
#[test]
fn a_stack_count_past_the_depth_is_refused() {
    let program = &[FnOp::PushInt(9), FnOp::Copy];
    assert_eq!(
        refuse(program),
        FunctionRefusal::StackCountOutOfRange {
            operator: "copy",
            count: 9,
            depth: 2,
        }
    );
}

/// §4.7 of the brief: a value that is not a number is refused loudly rather than turned into
/// NaN geometry — and WGSL's Finite Math Assumption would make it an arbitrary colour.
#[test]
fn a_non_finite_literal_is_refused() {
    let refusal = refuse(&[FnOp::PushReal(f32::NAN)]);
    assert!(
        matches!(refusal, FunctionRefusal::NonFiniteLiteral { at: 0, .. }),
        "{refusal}"
    );
    assert!(matches!(
        refuse(&[FnOp::PushReal(f32::INFINITY)]),
        FunctionRefusal::NonFiniteLiteral { at: 0, .. }
    ));
}

/// No static assignment of values to slots describes a join whose two arms left different
/// depths.
#[test]
fn an_ifelse_whose_arms_disagree_about_the_depth_is_refused() {
    // `x 0.5 gt { 1 1 } { 1 } ifelse`.
    let program = &[
        FnOp::Pop,
        FnOp::PushReal(0.5),
        FnOp::Gt,
        FnOp::JumpUnless { target: 7 },
        FnOp::PushReal(1.0),
        FnOp::PushReal(1.0),
        FnOp::Jump { target: 8 },
        FnOp::PushReal(1.0),
    ];
    assert_eq!(
        refuse(program),
        FunctionRefusal::UnbalancedBranches {
            at: 3,
            then_depth: 2,
            else_depth: 1,
        }
    );
}

/// PLRM3 requires integers of `idiv`, `mod` and `bitshift`, and gives `and`, `or`, `xor` and
/// `not` two readings neither of which covers a real. Lowering one over a real would truncate
/// it silently, which is a plausible colour no reading produces.
#[test]
fn a_type_sensitive_operator_over_a_real_is_refused() {
    let cases: [(&'static str, Vec<FnOp>); 4] = [
        (
            "idiv",
            vec![FnOp::PushReal(7.5), FnOp::PushInt(2), FnOp::Idiv],
        ),
        (
            "mod",
            vec![FnOp::PushReal(7.5), FnOp::PushInt(2), FnOp::Mod],
        ),
        (
            "bitshift",
            vec![FnOp::PushReal(7.5), FnOp::PushInt(2), FnOp::Bitshift],
        ),
        (
            "and",
            vec![FnOp::PushReal(7.5), FnOp::PushInt(2), FnOp::And],
        ),
    ];
    for (operator, program) in cases {
        let refusal = refuse(&program);
        assert!(
            matches!(
                refusal,
                FunctionRefusal::OperandType {
                    operator: named,
                    found: "a real",
                    ..
                } if named == operator
            ),
            "{operator}: {refusal}"
        );
    }

    // `not` over a real is the same rule, and it is the one the spike named.
    assert!(matches!(
        refuse(&[FnOp::PushReal(1.5), FnOp::Not]),
        FunctionRefusal::OperandType {
            operator: "not",
            found: "a real",
            ..
        }
    ));
}

/// A mixed boolean/integer pair is a `typecheck` in PostScript and has no single reading here
/// either — the operators are only defined over *two* booleans or *two* integers.
#[test]
fn a_mixed_boolean_and_integer_pair_is_refused() {
    let refusal = refuse(&[FnOp::PushBool(true), FnOp::PushInt(1), FnOp::And]);
    assert!(
        matches!(
            refusal,
            FunctionRefusal::OperandType {
                operator: "and",
                found: "an integer",
                ..
            }
        ),
        "{refusal}"
    );
}

/// §7.10.5.2's `if` and `ifelse` take a boolean. Lowering a real as `!= 0.0` would branch on a
/// value the standard never called a condition — and it is what would let an inexact operator
/// reach a branch without passing a comparison first.
#[test]
fn a_condition_that_is_not_a_boolean_is_refused() {
    // `x sin { 1 } { 0 } ifelse` — a real where a boolean belongs.
    let program = &[
        FnOp::Pop,
        FnOp::Sin,
        FnOp::JumpUnless { target: 5 },
        FnOp::PushReal(1.0),
        FnOp::Jump { target: 6 },
        FnOp::PushReal(0.0),
    ];
    assert_eq!(
        refuse(program),
        FunctionRefusal::OperandType {
            operator: "if",
            required: "a boolean",
            found: "a real",
        }
    );
}

/// `not` and `if` over a value two arms disagreed about — the spike's fifth ground, and now
/// the only way to reach an undecidable type at all, since the empty-stack zero has one.
#[test]
fn an_operand_of_no_decidable_type_is_refused() {
    // `x 0.5 gt { true } { 1 } ifelse` leaves a slot of no decidable type.
    let disagreed: &[FnOp] = &[
        FnOp::Pop,
        FnOp::PushReal(0.5),
        FnOp::Gt,
        FnOp::JumpUnless { target: 6 },
        FnOp::PushBool(true),
        FnOp::Jump { target: 7 },
        FnOp::PushInt(1),
    ];
    let with_not = [disagreed, &[FnOp::Not]].concat();
    assert_eq!(
        refuse(&with_not),
        FunctionRefusal::UndecidableOperandType { operator: "not" }
    );

    let with_branch = [disagreed, &[FnOp::JumpUnless { target: 9 }, FnOp::Pop]].concat();
    assert_eq!(
        refuse(&with_branch),
        FunctionRefusal::UndecidableOperandType { operator: "if" }
    );
}

/// A program can be perfectly good and still be refused for the shading that names it: the
/// `Range` is not the program's, so neither is this refusal.
///
/// ISO 32000-2 §7.10.5.3 makes the count rule an equality — "it shall be an error for the
/// number of remaining operands to differ" — so both directions are refused, and the
/// surplus direction is the one a device could otherwise draw by quietly reading the top
/// of the stack and discarding the rest.
#[test]
fn a_program_whose_output_count_differs_is_refused_at_admission() {
    let analysis = analyse(programs::SQRT_ALONE).unwrap();
    assert!(analysis.admits(programs::UNIT_GRAY).is_ok());
    assert_eq!(
        analysis.admits(programs::UNIT_RGB).unwrap_err(),
        FunctionRefusal::OutputCount {
            produced: 1,
            required: 3,
        }
    );

    // The other direction: three values under a one-component Range.
    let surplus = analyse(&[
        FnOp::Pop,
        FnOp::Pop,
        FnOp::PushInt(1),
        FnOp::PushInt(2),
        FnOp::PushInt(3),
    ])
    .unwrap();
    assert_eq!(
        surplus.admits(programs::UNIT_GRAY).unwrap_err(),
        FunctionRefusal::OutputCount {
            produced: 3,
            required: 1,
        }
    );
}

//! What the walk proves about a program: depth, slot types, resolved counts, and
//! `doc/adr/0053` §3's classification.
//!
//! One concern per test, and each names the property rather than the program.

mod function_support;

use function_support::programs::{self, UNIT_GRAY, UNIT_RGB, WIDE_GRAY, WIDE_RGB};
use quorra_gpu::function::{
    Agreement, FunctionRefusal, MAX_OPERAND_SLOTS, SlotType, Step, Unary, analyse,
};
use quorra_scene::{FnOp, FnRange};

/// §8.7.4.5.2 hands the function two coordinates, so the walk starts two deep and every slot
/// index the shader names is offset by them. The smallest program proves it: one push, and
/// the value lands in slot 2.
#[test]
fn the_two_inputs_occupy_the_first_two_slots() {
    let analysis = analyse(programs::CONSTANT_GREY).unwrap();
    assert_eq!(analysis.max_depth(), 3);
    assert_eq!(analysis.values_left(), 3);
    assert_eq!(analysis.steps().len(), 1);
}

/// The depth is the *greatest* the program reaches, not the depth it ends at — the shader
/// declares one `var` per slot and a program that grows and shrinks still used them.
#[test]
fn the_depth_is_the_peak_rather_than_the_end() {
    let analysis = analyse(programs::STACK_SHUFFLE).unwrap();
    // `dup`, then the two `roll` counts, reaches 5 before `roll` consumes them.
    assert_eq!(analysis.max_depth(), 5);
    assert_eq!(analysis.values_left(), 3);
    assert_eq!(analysis.slot_types().len(), 3);
}

/// Table 42's `not` is two operators wearing one name, and the operand's static type is the
/// only thing that says which. Both readings, from the same instruction.
#[test]
fn not_is_resolved_by_the_static_type_of_its_operand() {
    let integer = analyse(programs::NOT_ON_INTEGER).unwrap();
    assert!(matches!(
        integer.steps().last(),
        Some(Step::Unary {
            op: Unary::BitwiseNot,
            ..
        })
    ));

    let boolean = analyse(programs::NOT_ON_BOOLEAN).unwrap();
    assert!(matches!(
        boolean.steps().last(),
        Some(Step::Unary {
            op: Unary::LogicalNot,
            ..
        })
    ));
}

/// §7.10.5.2's arithmetic is integer-preserving where both operands are integers, and `div`
/// is "always a real number even if both operands are integers". The types are what `not` and
/// the integer family read, so they are asserted directly.
#[test]
fn arithmetic_preserves_the_integer_type_and_div_does_not() {
    let integers = analyse(&[FnOp::PushInt(6), FnOp::PushInt(4), FnOp::Add]).unwrap();
    assert_eq!(integers.slot_types().last(), Some(&SlotType::Integer));

    let divided = analyse(&[FnOp::PushInt(6), FnOp::PushInt(4), FnOp::Div]).unwrap();
    assert_eq!(divided.slot_types().last(), Some(&SlotType::Real));

    let compared = analyse(&[FnOp::PushInt(6), FnOp::PushInt(4), FnOp::Gt]).unwrap();
    assert_eq!(compared.slot_types().last(), Some(&SlotType::Boolean));
}

/// A value two arms of an `ifelse` disagree about has no static type — and the walk says so
/// rather than picking one, which is what makes the refusal in `function_refusals.rs`
/// possible.
#[test]
fn a_slot_two_arms_disagree_about_has_no_type() {
    // `x 0.5 gt { true } { 1 } ifelse` — a boolean on one arm and an integer on the other.
    let program = &[
        FnOp::Pop,
        FnOp::PushReal(0.5),
        FnOp::Gt,
        FnOp::JumpUnless { target: 6 },
        FnOp::PushBool(true),
        FnOp::Jump { target: 7 },
        FnOp::PushInt(1),
    ];
    let analysis = analyse(program).unwrap();
    assert_eq!(analysis.slot_types(), [SlotType::Undecided]);
}

/// The zero a pop from an empty operand stack yields is an **integer**, and the type is as
/// deliberate as the value: seven of Table 42's operators can tell an integer from a real, so
/// leaving it open would turn a decision about a value into a refusal about a type.
#[test]
fn the_empty_stack_zero_is_an_integer() {
    // `pop pop not` — the `not` reaches the zero the third pop yielded.
    let analysis = analyse(&[FnOp::Pop, FnOp::Pop, FnOp::Not]).unwrap();
    assert_eq!(analysis.slot_types(), [SlotType::Integer]);
    assert!(matches!(
        analysis.steps().last(),
        Some(Step::Unary {
            op: Unary::BitwiseNot,
            ..
        })
    ));
    assert_eq!(analysis.empty_stack_pops(), 1);
}

/// `doc/adr/0053` §3: a program that reaches only the exactly-agreeing operators is accepted
/// with the oracle relationship intact. No inexact operator, no amplification.
#[test]
fn a_program_without_an_inexact_operator_is_exact() {
    for (name, program) in [
        ("arithmetic only", programs::ARITHMETIC_ONLY),
        ("coordinates", programs::COORDINATES),
        // A discontinuity is not the danger; an *inexact operator upstream of* one is.
        ("discontinuous", programs::DISCONTINUOUS),
        ("integer ops", programs::INTEGER_OPS),
    ] {
        let analysis = analyse(program).unwrap();
        assert_eq!(analysis.agreement(), Agreement::Bounded, "{name}");
    }
}

/// And an inexact operator on its own is not the danger either: `sqrt` whose value reaches
/// nothing that amplifies it leaves the classification alone.
#[test]
fn an_inexact_operator_that_reaches_no_amplifier_is_exact() {
    for (name, program) in [
        ("sqrt alone", programs::SQRT_ALONE),
        ("sin, cos, exp", programs::TRANSCENDENTAL_A),
        ("ln, log, atan", programs::TRANSCENDENTAL_B),
    ] {
        let analysis = analyse(program).unwrap();
        assert_eq!(analysis.agreement(), Agreement::Bounded, "{name}");
    }
}

/// The composition is the danger, and the classification names both halves of it: which
/// operator may differ, and what turns the difference into a colour.
#[test]
fn an_inexact_operator_reaching_a_comparison_is_approximate() {
    let analysis = analyse(programs::SQRT_INTO_COMPARISON).unwrap();
    assert_eq!(
        analysis.agreement(),
        Agreement::Unbounded {
            inexact: "sqrt",
            inexact_at: 1,
            amplifier: "ge",
            amplifier_at: 3,
        }
    );
}

/// `truncate`, `cvi`, `round`, `floor` and `ceiling` are step functions, so they amplify for
/// the same reason a comparison does: a last-bit disagreement at an integer boundary becomes
/// a whole unit. `doc/adr/0053` §3 named only the comparisons; this is the extension, and the
/// reason it is one.
#[test]
fn a_rounding_operator_amplifies_like_a_comparison() {
    for (operator, op) in [
        ("truncate", FnOp::Truncate),
        ("cvi", FnOp::Cvi),
        ("round", FnOp::Round),
        ("floor", FnOp::Floor),
        ("ceiling", FnOp::Ceiling),
    ] {
        let analysis = analyse(&[FnOp::Pop, FnOp::Sqrt, op]).unwrap();
        assert_eq!(
            analysis.agreement(),
            Agreement::Unbounded {
                inexact: "sqrt",
                inexact_at: 1,
                amplifier: operator,
                amplifier_at: 2,
            },
            "{operator}"
        );
    }
}

/// The classification names the *first* inexact operator in the chain and the *first*
/// amplifier that reaches it, so that two runs over one program cannot name different ones.
#[test]
fn the_classification_names_the_earliest_pair() {
    // `x sqrt sin 0.5 ge` — two inexact operators before one comparison.
    let program = &[
        FnOp::Pop,
        FnOp::Sqrt,
        FnOp::Sin,
        FnOp::PushReal(0.5),
        FnOp::Ge,
    ];
    let analysis = analyse(program).unwrap();
    assert_eq!(
        analysis.agreement(),
        Agreement::Unbounded {
            inexact: "sqrt",
            inexact_at: 1,
            amplifier: "ge",
            amplifier_at: 4,
        }
    );
}

/// The pinned empty-stack decision is *counted*, so that wave 2 can raise the `Report`
/// ADR 0053 requires rather than adopting a plausible reading invisibly.
#[test]
fn a_program_that_pops_an_empty_stack_says_so() {
    let leaning = analyse(programs::EMPTY_STACK).unwrap();
    assert!(leaning.relies_on_empty_stack());
    assert_eq!(leaning.empty_stack_pops(), 2);

    let clean = analyse(programs::ARITHMETIC_ONLY).unwrap();
    assert!(!clean.relies_on_empty_stack());
    assert_eq!(clean.empty_stack_pops(), 0);
}

/// A `roll` of zero positions moves nothing, so it emits nothing: the walk drops a write whose
/// source is its own slot rather than making the shader prove it is a no-op.
#[test]
fn an_identity_permutation_costs_no_instruction() {
    let permutations = |steps: &[Step]| {
        steps
            .iter()
            .filter(|step| matches!(step, Step::Permute { .. }))
            .count()
    };

    let rolled = analyse(&[FnOp::PushInt(2), FnOp::PushInt(0), FnOp::Roll]).unwrap();
    assert_eq!(permutations(rolled.steps()), 0, "{:?}", rolled.steps());

    // `0 index` is not an identity: it copies the top to a *new* slot.
    let indexed = analyse(&[FnOp::PushInt(0), FnOp::Index]).unwrap();
    assert_eq!(permutations(indexed.steps()), 1);
}

/// The stated budget is discoverable before the frame, and the walk enforces it rather than
/// trusting a depth nobody computed.
#[test]
fn the_depth_budget_is_the_public_constant() {
    let mut program = vec![FnOp::Dup; MAX_OPERAND_SLOTS];
    // Two inputs plus one `dup` each: the budget is passed partway through.
    assert!(analyse(&program).is_err());
    program.truncate(MAX_OPERAND_SLOTS - 2 - 1);
    let analysis = analyse(&program).unwrap();
    assert!(analysis.max_depth() as usize <= MAX_OPERAND_SLOTS);
}

/// The `Range` is the shading's, not the program's, so it meets the program at
/// [`Analysis::admits`] rather than inside the walk. One uploaded program, two answers.
#[test]
fn a_range_meets_the_program_at_admission() {
    // Two values left: enough for `DeviceGray` and not for `DeviceRGB`.
    let analysis = analyse(&[FnOp::Pop, FnOp::PushReal(0.5)]).unwrap();
    assert_eq!(analysis.values_left(), 2);
    assert!(analysis.admits(UNIT_GRAY).is_ok());
    assert_eq!(
        analysis.admits(UNIT_RGB).unwrap_err(),
        FunctionRefusal::InsufficientOutputs {
            produced: 2,
            required: 3,
        }
    );

    let wide = analyse(programs::ARITHMETIC_ONLY).unwrap();
    assert!(wide.admits(WIDE_RGB).is_ok());
    assert!(wide.admits(WIDE_GRAY).is_ok());
}

/// Table 38 requires `min <= max`, and WGSL's `clamp` returns the *high* bound for every input
/// when the two are the other way round — a uniform wrong colour that looks like a colour.
#[test]
fn a_range_that_is_not_an_interval_is_refused() {
    let analysis = analyse(programs::CONSTANT_GREY).unwrap();
    assert_eq!(
        analysis.admits(FnRange::Gray([1.0, 0.0])).unwrap_err(),
        FunctionRefusal::RangeNotOrdered {
            component: 0,
            min: 1.0,
            max: 0.0,
        }
    );
    assert_eq!(
        analysis.admits(FnRange::Gray([0.0, f32::NAN])).unwrap_err(),
        FunctionRefusal::RangeNotFinite { component: 0 }
    );

    let rgb = analyse(programs::OUT_OF_RANGE).unwrap();
    assert_eq!(
        rgb.admits(FnRange::Rgb([[0.0, 1.0], [0.0, 1.0], [5.0, 1.0]]))
            .unwrap_err(),
        FunctionRefusal::RangeNotOrdered {
            component: 2,
            min: 5.0,
            max: 1.0,
        }
    );
}

/// The upload keys on the program and the pipeline keys on the shader, so the two hashes are
/// two hashes: one program under two ranges is one upload and two shaders.
#[test]
fn the_program_hash_and_the_shader_hash_are_different_questions() {
    let analysis = analyse(programs::OUT_OF_RANGE).unwrap();
    assert_eq!(
        analysis.shader_hash(UNIT_RGB),
        analysis.shader_hash(WIDE_RGB),
        "the range's numbers are runtime parameters and must not change the shader"
    );
    assert_ne!(
        analysis.shader_hash(UNIT_RGB),
        analysis.shader_hash(UNIT_GRAY)
    );
    assert_ne!(analysis.shader_hash(UNIT_RGB), analysis.program_hash());
}

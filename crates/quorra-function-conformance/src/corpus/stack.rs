//! Table 42's six stack operators.
//!
//! Every case here declares a three-component `Range`, because a stack shuffle is
//! easiest to state as a program that leaves three values and hardest to state as one
//! that leaves a single number — and because §7.10.5.3's rule that the count must match
//! is then doing work in every case rather than in one.
//!
//! `copy`, `index` and `roll` take their count *off the stack*, which is why they are
//! the three operators the refusal grounds name: a generated shader must resolve the
//! count at compile time or refuse the program. The cases below all use a literal count,
//! which is the case that must work; [`super::refusal`] holds the one that must not.

use crate::case::{Case, OPEN_RGB, PsError, Report, Subject};
use crate::table42::Table42;
use quorra_scene::function::FnOp as Op;

const fn about(operator: Table42) -> Subject {
    Subject::Operator(operator)
}

/// Every case in this family.
pub const CASES: &[Case] = &[
    Case::exact(
        "dup/duplicates-the-top",
        about(Table42::Dup),
        &[Op::PushInt(1), Op::PushInt(2), Op::Dup],
        &[1.0, 2.0, 2.0],
        "ISO 32000-2 Annex B.5: `any dup any any`, \"Duplicate top element\"; PLRM3 \
         ch. 8, `dup`.",
    )
    .with_range(OPEN_RGB),
    Case::exact(
        "exch/swaps-the-top-two",
        about(Table42::Exch),
        &[Op::PushInt(1), Op::PushInt(2), Op::PushInt(3), Op::Exch],
        &[1.0, 3.0, 2.0],
        "ISO 32000-2 Annex B.5: `any₁ any₂ exch any₂ any₁`; PLRM3 ch. 8, `exch`, \
         example `1 2 exch ⇒ 2 1`. The 1 underneath is untouched, which is the half of \
         the diagram an implementation can get wrong.",
    )
    .with_range(OPEN_RGB),
    Case::exact(
        "pop/discards-the-top",
        about(Table42::Pop),
        &[
            Op::PushInt(1),
            Op::PushInt(2),
            Op::PushInt(3),
            Op::PushInt(4),
            Op::Pop,
        ],
        &[1.0, 2.0, 3.0],
        "ISO 32000-2 Annex B.5: `any pop –`; PLRM3 ch. 8, `pop`, example `1 2 3 pop ⇒ \
         1 2`.",
    )
    .with_range(OPEN_RGB),
    Case::exact(
        "copy/one-element",
        about(Table42::Copy),
        &[Op::PushInt(1), Op::PushInt(2), Op::PushInt(1), Op::Copy],
        &[1.0, 2.0, 2.0],
        "ISO 32000-2 Annex B.5: `any₁ … anyₙ n copy any₁ … anyₙ any₁ … anyₙ`; PLRM3 \
         ch. 8, `copy`: \"copy pops n from the stack and duplicates the top n elements\".",
    )
    .with_range(OPEN_RGB),
    Case::exact(
        "copy/zero-elements-is-a-no-op",
        about(Table42::Copy),
        &[
            Op::PushInt(1),
            Op::PushInt(2),
            Op::PushInt(3),
            Op::PushInt(0),
            Op::Copy,
        ],
        &[1.0, 2.0, 3.0],
        "PLRM3 ch. 8, `copy`, example `(a) (b) (c) 0 copy ⇒ (a) (b) (c)`. A count of \
         zero is an ordinary case with its own example, not a degenerate one.",
    )
    .with_range(OPEN_RGB),
    Case::error(
        "copy/negative-count",
        about(Table42::Copy),
        &[Op::PushInt(1), Op::PushInt(2), Op::PushInt(-1), Op::Copy],
        PsError::RangeCheck,
        "PLRM3 ch. 8, `copy`: the first form applies \"where the top element on the \
         operand stack is a nonnegative integer n\"; Errors: `rangecheck`. Which listed \
         error a negative count raises is our reading — the entry states the \
         restriction and lists the errors without joining them.",
    )
    .with_range(OPEN_RGB),
    Case::exact(
        "index/one-below-the-top",
        about(Table42::Index),
        &[Op::PushInt(10), Op::PushInt(20), Op::PushInt(1), Op::Index],
        &[10.0, 20.0, 10.0],
        "ISO 32000-2 Annex B.5: `anyₙ … any₀ n index anyₙ … any₀ anyₙ`; PLRM3 ch. 8, \
         `index`, example `(a) (b) (c) (d) 3 index ⇒ (a) (b) (c) (d) (a)`. The count is \
         a distance from the top, so 1 reaches the element below it.",
    )
    .with_range(OPEN_RGB),
    Case::exact(
        "index/zero-is-the-top",
        about(Table42::Index),
        &[Op::PushInt(10), Op::PushInt(20), Op::PushInt(0), Op::Index],
        &[10.0, 20.0, 20.0],
        "PLRM3 ch. 8, `index`, example `(a) (b) (c) (d) 0 index ⇒ (a) (b) (c) (d) (d)`; \
         `0 index` is `dup`.",
    )
    .with_range(OPEN_RGB),
    Case::error(
        "index/past-the-bottom",
        about(Table42::Index),
        &[Op::PushInt(10), Op::PushInt(20), Op::PushInt(5), Op::Index],
        PsError::RangeCheck,
        "PLRM3 ch. 8, `index`, Errors: `rangecheck`. The entry says the operator \"counts \
         down to the nth element from the top of the stack\" and there is no such \
         element; the identification of the error is our reading of the entry's list.",
    )
    .with_range(OPEN_RGB),
    Case::exact(
        "roll/positive-count-moves-up",
        about(Table42::Roll),
        &[
            Op::PushInt(1),
            Op::PushInt(2),
            Op::PushInt(3),
            Op::PushInt(3),
            Op::PushInt(1),
            Op::Roll,
        ],
        &[3.0, 1.0, 2.0],
        "ISO 32000-2 Annex B.5: `anyₙ₋₁ … any₀ n j roll any₍ⱼ₋₁₎ mod n … any₀ anyₙ₋₁ … \
         anyⱼ mod n`, \"Roll n elements up j times\"; PLRM3 ch. 8, `roll`: \"Positive j \
         indicates upward motion on the stack\", each shift \"removing an element from \
         the top of the stack and inserting it between element n − 1 and element n\".",
    )
    .with_range(OPEN_RGB),
    Case::exact(
        "roll/negative-count-moves-down",
        about(Table42::Roll),
        &[
            Op::PushInt(1),
            Op::PushInt(2),
            Op::PushInt(3),
            Op::PushInt(3),
            Op::PushInt(-1),
            Op::Roll,
        ],
        &[2.0, 3.0, 1.0],
        "PLRM3 ch. 8, `roll`: \"negative j indicates downward motion\". Derived from the \
         entry's sentence and its stack diagram; the entry prints no numeric example, \
         and the two directions are exactly what an implementation transposes.",
    )
    .with_range(OPEN_RGB),
    Case::exact(
        "roll/zero-elements-is-a-no-op",
        about(Table42::Roll),
        &[
            Op::PushInt(1),
            Op::PushInt(2),
            Op::PushInt(3),
            Op::PushInt(0),
            Op::PushInt(7),
            Op::Roll,
        ],
        &[1.0, 2.0, 3.0],
        "PLRM3 ch. 8, `roll`: \"n must be a nonnegative integer and j must be an \
         integer\", and a circular shift of no elements moves nothing whatever j is. \
         Derived; the entry gives no example, and Annex B's `any₍ⱼ₋₁₎ mod n` is \
         undefined at n = 0, which is why the case is here rather than assumed.",
    )
    .with_range(OPEN_RGB),
    Case::exact(
        "empty-stack-pop/yields-zero-and-is-reported",
        Subject::EmptyStackPop,
        &[Op::PushInt(5), Op::Add],
        &[5.0],
        "**No clause defines this.** PostScript raises `stackunderflow` (PLRM3 lists it \
         under every operator that pops), and ISO 32000-2 says nothing about a program \
         that pops more than it pushed. The pinned vocabulary's decision 6 takes the \
         caller's reading — 0 — because their `pi_seven_segment.pdf` depends on it three \
         times and `doc/spike-function-paint.md` §7 traces the consequence into the \
         picture. The expected value is therefore **a decision, not a derivation**, and \
         the case carries the report that says so.",
    )
    .with_report(Report::EmptyStackPop),
];

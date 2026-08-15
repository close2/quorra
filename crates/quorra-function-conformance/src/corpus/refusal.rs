//! One program per refusal ground, each of which actually reaches it.
//!
//! `doc/spike-function-paint.md` §6 states the rule these cases exist to satisfy: "each
//! is demonstrated on a constructed program that reaches it, because a ground nobody can
//! reach is not a ground". Principle 6 is why they are refusals and not approximations —
//! a program we cannot lower is an `Err` that names what it could not do, never a
//! plausible colour.
//!
//! Three of the seven cannot be reached by *running* a program, and that is the finding
//! rather than a gap: a count that came off the stack is perfectly resolvable at run
//! time, a join is only ever reached down one arm, and a type ambiguity is a property of
//! two paths rather than of the one taken. [`Refusal::reached_by_evaluation`] says which
//! is which, and the corpus's test asserts exactly that split — so the three static
//! grounds are a promissory note the analyser has to redeem, and the corpus says so out
//! loud instead of pretending the evaluator checks them.
//!
//! Two grounds the spike listed are **not** here, because the pinned vocabulary retired
//! them: "an operator outside Table 42" and "a procedure that is not an `if`/`ifelse`
//! operand" are both unrepresentable in `FnOp`, so no program can reach them and no
//! refusal is needed. That is a type system doing a validator's job, and it is worth
//! knowing which grounds it retired: the spike's six became four for that reason, and
//! three more join them here — a backward jump, a target past the end, and an output
//! count that cannot match `Range`.

use crate::case::{Case, OPEN_RGB, Refusal};
use quorra_scene::function::FnOp as Op;

/// 101 pushes against §7.10.5.3's 100-entry stack: one more than the clause obliges any
/// implementation to provide.
const DEEPER_THAN_THE_CLAUSE_REQUIRES: [Op; 101] = [Op::PushInt(1); 101];

/// Every case in this family.
pub const CASES: &[Case] = &[
    Case::refused(
        "refusal/operand-stack-deeper-than-one-hundred",
        &DEEPER_THAN_THE_CLAUSE_REQUIRES,
        Refusal::OperandStackTooDeep,
        "ISO 32000-2 §7.10.5.3: \"Implementations of Type 4 functions shall provide a \
         stack with room for at least 100 entries. No implementation shall be required \
         to provide a larger stack, and it shall be an error to overflow the stack.\" So \
         the limit is normative, a program needing 101 entries may be refused by any \
         conforming processor, and §5 of the brief's rule applies: the limit is \
         discoverable before the frame rather than after it.",
    ),
    Case::refused(
        "refusal/backward-jump",
        &[Op::PushInt(1), Op::Jump { target: 0 }],
        Refusal::BackwardJump,
        "The pinned vocabulary's decision 2, and the reason `doc/spike-function-paint.md` \
         §4 could bound the interpreter's loop: forward-only jumps make the instruction \
         count the execution bound. Nothing in ISO 32000-2 forbids a loop, because \
         §7.10.5's syntax cannot express one — `if` and `ifelse` are the only control \
         flow — so a backward jump means the compiled form is not a lowering of any \
         legal function.",
    ),
    Case::refused(
        "refusal/jump-past-the-end",
        &[Op::PushInt(1), Op::Jump { target: 9 }],
        Refusal::JumpOutOfRange,
        "As the backward jump: a target past the end is not a lowering of any §7.10.5 \
         program. A target *equal* to the length is legal and means halt — \
         [`super::conditional`]'s cases depend on it — which is exactly why the bound is \
         `target <= len` and worth a case of its own.",
    ),
    Case::refused(
        "refusal/output-count-cannot-match-the-range",
        &[Op::PushInt(1), Op::PushInt(2), Op::PushInt(3)],
        Refusal::OutputCountMismatch,
        "ISO 32000-2 §7.10.5.3's count rule again, but statically: this program leaves \
         three values on every path and the paint declares one component, so no input \
         makes it legal. §5 of the brief asks for limits that are discoverable before \
         the frame, and a program that cannot ever produce the right number of outputs \
         is the cheapest such refusal there is.",
    ),
    Case::refused(
        "refusal/stack-count-that-is-not-a-literal",
        &[Op::PushInt(99), Op::Exch, Op::Cvi, Op::Index],
        Refusal::NonLiteralStackCount,
        "`doc/spike-function-paint.md` §6: \"copy/index/roll whose count is not a literal \
         — shape (ii) cannot name a slot it cannot compute, and shape (i) cannot state \
         its own depth\". Here the count reaching `index` is the shading's own y \
         coordinate through `cvi`, so it is different at every fragment. **The reference \
         evaluator computes it happily** — with these inputs it is 1 — which is the \
         point: this ground is visible only to a static walk, and running the program is \
         no evidence at all.",
    )
    .with_inputs(&[5.0, 1.0])
    .with_domain([0.0, 10.0, 0.0, 10.0])
    .with_range(OPEN_RGB),
    Case::refused(
        "refusal/branches-join-at-different-depths",
        &[
            Op::PushInt(0),
            Op::PushBool(true),
            Op::JumpUnless { target: 6 },
            Op::PushInt(1),
            Op::PushInt(2),
            Op::Jump { target: 7 },
            Op::PushInt(3),
        ],
        Refusal::JoinDepthMismatch,
        "`doc/spike-function-paint.md` §6: \"an `ifelse` whose arms leave different \
         depths — no static slot assignment describes the join\". The true arm leaves \
         three values and the false arm two. Only one arm runs, so the evaluator returns \
         three outputs and sees nothing wrong; a generated shader has to name the slots \
         at the join and cannot.",
    )
    .with_range(OPEN_RGB),
    Case::refused(
        "refusal/type-two-branches-disagree-about",
        &[
            Op::PushBool(true),
            Op::JumpUnless { target: 4 },
            Op::PushInt(63),
            Op::Jump { target: 5 },
            Op::PushBool(true),
            Op::Not,
        ],
        Refusal::AmbiguousOperandType,
        "`doc/spike-function-paint.md` §6: \"`not` on a value two branches disagreed \
         about\". Table 42's `not` is two operators (PLRM3: logical on a boolean, ones \
         complement on an integer), and here one arm leaves an integer and the other a \
         boolean, so *which operator this occurrence is* has no static answer. The \
         evaluator takes the integer arm and returns −64 without noticing; the analyser \
         must see both.",
    ),
];

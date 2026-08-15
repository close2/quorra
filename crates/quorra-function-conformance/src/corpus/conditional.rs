//! `if` and `ifelse`, in the form they actually reach us.
//!
//! ISO 32000-2 §7.10.5.2 writes them syntactically —
//!
//! > - *boolean* {*expression*} **if**
//! > - *boolean* {*expression₁*} {*expression₂*} **ifelse**
//! >
//! > This construct is purely syntactic; unlike in the PostScript language, no
//! > "procedure objects" shall be involved.
//!
//! — and the pinned vocabulary's decision 1 has the caller lower the braces into
//! [`FnOp::JumpUnless`](quorra_scene::function::FnOp::JumpUnless) and
//! [`FnOp::Jump`](quorra_scene::function::FnOp::Jump) before we see them. So the cases
//! below are `if` and `ifelse`, and the thing they check is that the lowering means what
//! the clause means: the condition is consumed, exactly one arm runs, and a jump target
//! equal to the program's length halts.
//!
//! Every jump here is forward, which is not a property of these examples but the
//! condition on the whole vocabulary: it is what makes the instruction count an
//! execution bound. A backward jump has a case, in [`super::refusal`].

use crate::case::{Case, Subject};
use crate::table42::Table42;
use quorra_scene::function::FnOp as Op;

const fn about(operator: Table42) -> Subject {
    Subject::Operator(operator)
}

/// Every case in this family.
pub const CASES: &[Case] = &[
    Case::exact(
        "if/condition-true-runs-the-arm",
        about(Table42::If),
        &[
            Op::PushInt(7),
            Op::PushInt(3),
            Op::PushInt(4),
            Op::Lt,
            Op::JumpUnless { target: 7 },
            Op::PushInt(1),
            Op::Add,
        ],
        &[8.0],
        "`7 3 4 lt { 1 add } if`. PLRM3 ch. 8, `if`: \"removes both operands from the \
         stack, then executes proc if bool is true\", and `3 4 lt` is the entry's own \
         condition. The jump target is 7, which is the program's length: an arm that \
         runs to the end needs the target to be one past the last instruction.",
    ),
    Case::exact(
        "if/condition-false-skips-the-arm",
        about(Table42::If),
        &[
            Op::PushInt(7),
            Op::PushInt(4),
            Op::PushInt(3),
            Op::Lt,
            Op::JumpUnless { target: 7 },
            Op::PushInt(1),
            Op::Add,
        ],
        &[7.0],
        "The same program with the comparison reversed. PLRM3 ch. 8, `if`: the operator \
         \"pushes no results of its own\", so a false condition leaves the stack exactly \
         as the condition found it — the 7 that was under it, and nothing else.",
    ),
    Case::exact(
        "ifelse/condition-false-runs-the-second-arm",
        about(Table42::Ifelse),
        &[
            Op::PushInt(4),
            Op::PushInt(3),
            Op::Lt,
            Op::JumpUnless { target: 6 },
            Op::PushInt(10),
            Op::Jump { target: 7 },
            Op::PushInt(20),
        ],
        &[20.0],
        "PLRM3 ch. 8, `ifelse`, which is exactly this program in its own example: `4 3 \
         lt {(TruePart)} {(FalsePart)} ifelse ⇒ (FalsePart)  % Since 4 is not less than \
         3`.",
    ),
    Case::exact(
        "ifelse/condition-true-runs-the-first-arm",
        about(Table42::Ifelse),
        &[
            Op::PushInt(3),
            Op::PushInt(4),
            Op::Lt,
            Op::JumpUnless { target: 6 },
            Op::PushInt(10),
            Op::Jump { target: 7 },
            Op::PushInt(20),
        ],
        &[10.0],
        "PLRM3 ch. 8, `ifelse`: \"executes proc1 if bool is true or proc2 if bool is \
         false\". The `Jump` at index 5 is what makes the arms exclusive; without it the \
         true arm falls into the false one and the program leaves two values, which \
         §7.10.5.3 makes an error.",
    ),
];

//! Table 42's relational, boolean and bitwise operators, and the two questions of type
//! hiding inside them.
//!
//! §7.10.5.3 forbids a boolean as an *output* — "It shall be an error … for any of them
//! to be objects other than numbers" — so every case that ends in a comparison converts
//! it with a lowered `ifelse`, which is the shape a caller hands us anyway. The tail
//! `JumpUnless{k+4}; 1; Jump{k+5}; 0` appears throughout and means exactly `{1} {0}
//! ifelse`.
//!
//! Two cases here are the ones an implementation gets wrong without noticing:
//!
//! - **`not` on an integer.** Table 42's `not` is one name over two operators, and
//!   `63 not` is −64 rather than 0. An evaluator whose literals carry no type implements
//!   the boolean one only and answers every integer case wrongly.
//! - **`eq` has no tolerance.** PLRM3 describes equality of values with a numeric
//!   coercion and nothing else; `0.5` and the next representable float above it are not
//!   equal, and an epsilon comparison says they are.

use crate::case::{Case, PsError, Subject};
use crate::table42::Table42;
use quorra_scene::function::FnOp as Op;

const fn about(operator: Table42) -> Subject {
    Subject::Operator(operator)
}

/// Every case in this family.
pub const CASES: &[Case] = &[
    // ---- eq, ne -------------------------------------------------------------
    Case::exact(
        "eq/integer-equals-real-of-the-same-value",
        about(Table42::Eq),
        &[
            Op::PushInt(1),
            Op::PushReal(1.0),
            Op::Eq,
            Op::JumpUnless { target: 6 },
            Op::PushInt(1),
            Op::Jump { target: 7 },
            Op::PushInt(0),
        ],
        &[1.0],
        "PLRM3 ch. 8, `eq`: \"Integers and real numbers can be compared freely: an \
         integer and a real number representing the same mathematical value are \
         considered equal by eq.\" So the type rule that separates `1` from `1.0` \
         everywhere else does not apply here.",
    ),
    Case::exact(
        "eq/boolean-is-never-equal-to-a-number",
        about(Table42::Eq),
        &[
            Op::PushBool(true),
            Op::PushInt(1),
            Op::Eq,
            Op::JumpUnless { target: 6 },
            Op::PushInt(1),
            Op::Jump { target: 7 },
            Op::PushInt(0),
        ],
        &[0.0],
        "PLRM3 ch. 8, `eq`: \"Simple objects are equal if their types and values are the \
         same\", and the only conversion the entry offers is between integers and reals. \
         `eq` takes `any₁ any₂`, so this is false rather than a `typecheck`. An \
         implementation that represents `true` as 1.0 answers 1 here.",
    ),
    Case::exact(
        "eq/has-no-tolerance",
        about(Table42::Eq),
        &[
            Op::PushReal(0.5),
            Op::PushReal(0.500_000_06),
            Op::Eq,
            Op::JumpUnless { target: 6 },
            Op::PushInt(1),
            Op::Jump { target: 7 },
            Op::PushInt(0),
        ],
        &[0.0],
        "PLRM3 ch. 8, `eq`, which describes equality of values and names no tolerance. \
         The two operands are adjacent binary32 values, so they differ by 5.96e−8 — less \
         than `f32::EPSILON` — and an evaluator comparing with `(a − b).abs() < EPSILON` \
         answers 1. Research §3.3 records that the caller's own evaluator does exactly \
         that; this case is the clause, not their source.",
    ),
    Case::exact(
        "ne/differing-integers",
        about(Table42::Ne),
        &[
            Op::PushInt(1),
            Op::PushInt(2),
            Op::Ne,
            Op::JumpUnless { target: 6 },
            Op::PushInt(1),
            Op::Jump { target: 7 },
            Op::PushInt(0),
        ],
        &[1.0],
        "PLRM3 ch. 8, `ne`: \"pushes false if they are equal, or true if not. What it \
         means for objects to be equal is presented in the description of the eq \
         operator.\"",
    ),
    // ---- gt, ge, lt, le -----------------------------------------------------
    Case::exact(
        "gt/greater",
        about(Table42::Gt),
        &[
            Op::PushInt(4),
            Op::PushInt(3),
            Op::Gt,
            Op::JumpUnless { target: 6 },
            Op::PushInt(1),
            Op::Jump { target: 7 },
            Op::PushInt(0),
        ],
        &[1.0],
        "PLRM3 ch. 8, `gt`: \"pushes true if the first operand is greater than the \
         second\".",
    ),
    Case::exact(
        "gt/equal-is-not-greater",
        about(Table42::Gt),
        &[
            Op::PushInt(3),
            Op::PushInt(3),
            Op::Gt,
            Op::JumpUnless { target: 6 },
            Op::PushInt(1),
            Op::Jump { target: 7 },
            Op::PushInt(0),
        ],
        &[0.0],
        "PLRM3 ch. 8, `gt`, by the same sentence: the boundary belongs to `ge`.",
    ),
    Case::error(
        "gt/boolean-operands",
        about(Table42::Gt),
        &[Op::PushBool(true), Op::PushBool(false), Op::Gt],
        PsError::TypeCheck,
        "PLRM3 ch. 8, `gt`: \"If the operands are of other types … a typecheck error \
         occurs.\" The four order comparisons take `num₁ num₂` (or two strings, which \
         §7.10.5.1 excludes); an implementation that orders `true` after `false` has \
         invented an operator.",
    ),
    Case::exact(
        "ge/greater-or-equal",
        about(Table42::Ge),
        &[
            Op::PushReal(4.2),
            Op::PushInt(4),
            Op::Ge,
            Op::JumpUnless { target: 6 },
            Op::PushInt(1),
            Op::Jump { target: 7 },
            Op::PushInt(0),
        ],
        &[1.0],
        "PLRM3 ch. 8, `ge`, example `4.2 4 ge ⇒ true`; note that it compares \"their \
         mathematical values\" across the two numeric types.",
    ),
    Case::exact(
        "lt/less",
        about(Table42::Lt),
        &[
            Op::PushInt(3),
            Op::PushInt(4),
            Op::Lt,
            Op::JumpUnless { target: 6 },
            Op::PushInt(1),
            Op::Jump { target: 7 },
            Op::PushInt(0),
        ],
        &[1.0],
        "PLRM3 ch. 8, `lt`: \"pushes true if the first operand is less than the second\". \
         `3 4 lt` is the condition in PLRM3's own `if` example.",
    ),
    Case::exact(
        "le/equal-is-less-or-equal",
        about(Table42::Le),
        &[
            Op::PushInt(3),
            Op::PushInt(3),
            Op::Le,
            Op::JumpUnless { target: 6 },
            Op::PushInt(1),
            Op::Jump { target: 7 },
            Op::PushInt(0),
        ],
        &[1.0],
        "PLRM3 ch. 8, `le`: \"pushes true if the first operand is less than or equal to \
         the second\".",
    ),
    // ---- and, or, xor -------------------------------------------------------
    Case::exact(
        "and/booleans",
        about(Table42::And),
        &[
            Op::PushBool(true),
            Op::PushBool(false),
            Op::And,
            Op::JumpUnless { target: 6 },
            Op::PushInt(1),
            Op::Jump { target: 7 },
            Op::PushInt(0),
        ],
        &[0.0],
        "PLRM3 ch. 8, `and`, truth table: `true false and ⇒ false`.",
    ),
    Case::exact(
        "and/integers-are-bitwise",
        about(Table42::And),
        &[Op::PushInt(52), Op::PushInt(7), Op::And],
        &[4.0],
        "PLRM3 ch. 8, `and`, example `52 7 and ⇒ 4`: \"If the operands are integers, and \
         returns the bitwise 'and' of their binary representations.\"",
    ),
    Case::error(
        "and/one-boolean-and-one-integer",
        about(Table42::And),
        &[Op::PushBool(true), Op::PushInt(1), Op::And],
        PsError::TypeCheck,
        "PLRM3 ch. 8, `and`, whose operand rows are `bool₁ bool₂` and `int₁ int₂` and \
         admit no mixture; Errors: `typecheck`. Our reading of the two rows, and the \
         reason the corpus states it: with `true` as 1 the two readings agree \
         everywhere else, which is why an implementation that conflates them passes \
         every other `and` case.",
    ),
    Case::exact(
        "or/integers-are-bitwise",
        about(Table42::Or),
        &[Op::PushInt(17), Op::PushInt(5), Op::Or],
        &[21.0],
        "PLRM3 ch. 8, `or`, example `17 5 or ⇒ 21`.",
    ),
    Case::exact(
        "or/booleans",
        about(Table42::Or),
        &[
            Op::PushBool(false),
            Op::PushBool(false),
            Op::Or,
            Op::JumpUnless { target: 6 },
            Op::PushInt(1),
            Op::Jump { target: 7 },
            Op::PushInt(0),
        ],
        &[0.0],
        "PLRM3 ch. 8, `or`, truth table: `false false or ⇒ false`.",
    ),
    Case::exact(
        "xor/integers-are-bitwise",
        about(Table42::Xor),
        &[Op::PushInt(12), Op::PushInt(3), Op::Xor],
        &[15.0],
        "PLRM3 ch. 8, `xor`, example `12 3 xor ⇒ 15`.",
    ),
    Case::exact(
        "xor/booleans",
        about(Table42::Xor),
        &[
            Op::PushBool(true),
            Op::PushBool(true),
            Op::Xor,
            Op::JumpUnless { target: 6 },
            Op::PushInt(1),
            Op::Jump { target: 7 },
            Op::PushInt(0),
        ],
        &[0.0],
        "PLRM3 ch. 8, `xor`, truth table: `true true xor ⇒ false`.",
    ),
    // ---- not ----------------------------------------------------------------
    Case::exact(
        "not/boolean",
        about(Table42::Not),
        &[
            Op::PushBool(true),
            Op::Not,
            Op::JumpUnless { target: 5 },
            Op::PushInt(1),
            Op::Jump { target: 6 },
            Op::PushInt(0),
        ],
        &[0.0],
        "PLRM3 ch. 8, `not`, truth table: `true not ⇒ false`.",
    ),
    Case::exact(
        "not/integer-is-ones-complement",
        about(Table42::Not),
        &[Op::PushInt(52), Op::Not],
        &[-53.0],
        "PLRM3 ch. 8, `not`, example `52 not ⇒ −53`: \"If the operand is an integer, not \
         returns the bitwise complement (ones complement) of its binary representation.\"",
    ),
    Case::exact(
        "not/integer-sixty-three",
        about(Table42::Not),
        &[Op::PushInt(63), Op::Not],
        &[-64.0],
        "PLRM3 ch. 8, `not`, by the same sentence: ¬63 = −64 in two's complement. This \
         is the case `doc/spike-function-paint.md` §7 found the caller's evaluator \
         answering with 0.0, because its compiled form carries no type on a literal — \
         which is why the pinned vocabulary has both `PushInt` and `PushReal`.",
    ),
    Case::error(
        "not/real-operand",
        about(Table42::Not),
        &[Op::PushReal(4.5), Op::Not],
        PsError::TypeCheck,
        "PLRM3 ch. 8, `not`, whose operand rows are `bool₁` and `int₁` and nothing else; \
         Errors: `typecheck`. The reading is ours in the same way as `and`'s: the entry \
         lists the error and the rows, and joining them is the step.",
    ),
    // ---- bitshift -----------------------------------------------------------
    Case::exact(
        "bitshift/left",
        about(Table42::Bitshift),
        &[Op::PushInt(7), Op::PushInt(3), Op::Bitshift],
        &[56.0],
        "PLRM3 ch. 8, `bitshift`, example `7 3 bitshift ⇒ 56`.",
    ),
    Case::exact(
        "bitshift/negative-count-shifts-right",
        about(Table42::Bitshift),
        &[Op::PushInt(142), Op::PushInt(-3), Op::Bitshift],
        &[17.0],
        "PLRM3 ch. 8, `bitshift`, example `142 –3 bitshift ⇒ 17`: \"If shift is \
         negative, a right shift by –shift bits is performed.\"",
    ),
    Case::exact(
        "bitshift/right-shift-of-a-negative-operand-fills-zeros",
        about(Table42::Bitshift),
        &[Op::PushInt(-8), Op::PushInt(-28), Op::Bitshift],
        &[15.0],
        "PLRM3 ch. 8, `bitshift`: \"shifts the binary representation of int1 … Bits \
         shifted out are lost; bits shifted in are 0. […] This operation produces an \
         arithmetically correct result only for positive values of int1.\" Zeros shifted \
         in makes the right shift a *logical* one, and the warning about positive \
         operands is what confirms it — an arithmetic shift would need no such warning. \
         −8 is 0xFFFFFFF8, so a logical shift by 28 gives 15. **Rust's `>>` and WGSL's \
         `>>` on a signed integer are both arithmetic and give −1.**",
    ),
    Case::undefined(
        "bitshift/count-at-the-operand-width",
        about(Table42::Bitshift),
        &[Op::PushInt(1), Op::PushInt(32), Op::Bitshift],
        "PLRM3's `bitshift` entry does not say what a shift of 32 or more places \
         produces, and ISO 32000-2 Annex B says only \"Perform bitwise shift of int₁ \
         (positive is left)\"; WGSL §8.7 takes the count modulo the bit width, which \
         would return the operand unchanged, and Rust's `<<` overflows",
        "PLRM3 ch. 8, `bitshift`. \"Bits shifted out are lost\" suggests 0, but the \
         entry never says so, and the three plausible answers — 0, the operand back, and \
         a panic — are all somebody's implementation. Research §5 already recorded this \
         as unverified; the corpus records it as undefined.",
    ),
    // ---- true, false --------------------------------------------------------
    Case::exact(
        "true/pushes-a-boolean",
        about(Table42::True),
        &[
            Op::PushBool(true),
            Op::JumpUnless { target: 4 },
            Op::PushInt(1),
            Op::Jump { target: 5 },
            Op::PushInt(0),
        ],
        &[1.0],
        "PLRM3 ch. 8, `true`: \"pushes a boolean object whose value is true on the \
         operand stack\". It has to be observed through a branch, because §7.10.5.3 \
         forbids a boolean output.",
    ),
    Case::exact(
        "false/pushes-a-boolean",
        about(Table42::False),
        &[
            Op::PushBool(false),
            Op::JumpUnless { target: 4 },
            Op::PushInt(1),
            Op::Jump { target: 5 },
            Op::PushInt(0),
        ],
        &[0.0],
        "PLRM3 ch. 8, `false`: \"pushes a boolean object whose value is false\".",
    ),
];

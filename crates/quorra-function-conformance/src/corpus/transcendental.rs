//! The seven operators whose last bit no specification fixes: `atan sin cos exp ln log
//! sqrt`.
//!
//! Their *values* are defined — an arc tangent is an arc tangent — but nothing in
//! ISO 32000-2 or PLRM3 states how accurately a processor must compute one, and WGSL's
//! §15.7.4.1 states bounds that are wide on purpose: 4 096 ULP for `atan`, an absolute
//! 2⁻¹¹ for `sin` and `cos` **only inside ±π radians**, and nothing at all outside.
//! `doc/adr/0053` measured the consequence on this machine's two adapters over 4 096
//! inputs: `sin` and `cos` differ on 3 201 and 3 334 of them, `exp` on 2 660, `sqrt` on
//! 618, `atan` on 375.
//!
//! So every expectation here is a [`Tolerance::Absolute`](crate::Tolerance::Absolute),
//! and the bound is the *test's* instrument — PLRM3 prints its examples to six digits —
//! rather than a promise anyone made. What the corpus pins exactly is the shape of each
//! operator: that `atan` returns **degrees in 0..360** and not radians, that `sin` and
//! `cos` take degrees, that `exp` accepts a negative base with an integral exponent
//! where `pow` does not, and where each one has no value at all.

// Two expected values below are PLRM3's printed decimals for √2 and ln 10, and clippy
// offers `f32::consts::SQRT_2` and `LN_10` instead. Taking them would replace the
// document's number with Rust's, which is the substitution principle 5 forbids: the
// expectation is what the entry prints, to the digits it prints.
#![allow(clippy::approx_constant)]

use crate::case::{Case, PsError, Subject};
use crate::table42::Table42;
use quorra_scene::function::FnOp as Op;

const fn about(operator: Table42) -> Subject {
    Subject::Operator(operator)
}

/// Every case in this family.
pub const CASES: &[Case] = &[
    // ---- atan ---------------------------------------------------------------
    Case::near(
        "atan/zero-over-one",
        about(Table42::Atan),
        &[Op::PushInt(0), Op::PushInt(1), Op::Atan],
        &[0.0],
        1.0e-4,
        "PLRM3 ch. 8, `atan`, example `0 1 atan ⇒ 0.0`.",
    ),
    Case::near(
        "atan/one-over-zero-is-ninety-degrees",
        about(Table42::Atan),
        &[Op::PushInt(1), Op::PushInt(0), Op::Atan],
        &[90.0],
        1.0e-4,
        "PLRM3 ch. 8, `atan`, example `1 0 atan ⇒ 90.0`: \"returns the angle (in degrees \
         between 0 and 360) whose tangent is num divided by den\". Degrees, not radians \
         — a radian answer here would be 1.5708.",
    ),
    Case::near(
        "atan/third-quadrant-wraps-to-270",
        about(Table42::Atan),
        &[Op::PushInt(-100), Op::PushInt(0), Op::Atan],
        &[270.0],
        1.0e-4,
        "PLRM3 ch. 8, `atan`, example `−100 0 atan ⇒ 270.0`. The range is 0..360, so a \
         two-argument arc tangent returning −180..180 is wrong by a full turn here; \
         this is the case that catches a bare `atan2`.",
    ),
    Case::near(
        "atan/forty-five-degrees",
        about(Table42::Atan),
        &[Op::PushInt(4), Op::PushInt(4), Op::Atan],
        &[45.0],
        1.0e-4,
        "PLRM3 ch. 8, `atan`, example `4 4 atan ⇒ 45.0`.",
    ),
    Case::error(
        "atan/both-operands-zero",
        about(Table42::Atan),
        &[Op::PushInt(0), Op::PushInt(0), Op::Atan],
        PsError::UndefinedResult,
        "PLRM3 ch. 8, `atan`: \"Either num or den may be 0, but not both\"; Errors: \
         `undefinedresult`. Rust's `f32::atan2(0.0, 0.0)` returns 0.0 and WGSL's \
         `atan2` is undefined for it, so this is a guard a lowering must add rather than \
         a case the host gets right.",
    ),
    // ---- sin ----------------------------------------------------------------
    Case::near(
        "sin/zero-degrees",
        about(Table42::Sin),
        &[Op::PushInt(0), Op::Sin],
        &[0.0],
        1.0e-6,
        "PLRM3 ch. 8, `sin`: \"returns the sine of angle, which is interpreted as an \
         angle in degrees\".",
    ),
    Case::near(
        "sin/thirty-degrees-is-a-half",
        about(Table42::Sin),
        &[Op::PushInt(30), Op::Sin],
        &[0.5],
        1.0e-6,
        "PLRM3 ch. 8, `sin`, with the operand read as degrees: sin 30° = 1/2 exactly. \
         An implementation that read the operand as radians would give 0.988. The value \
         is mathematics; the bound is the test's.",
    ),
    Case::near(
        "sin/three-hundred-sixty-degrees",
        about(Table42::Sin),
        &[Op::PushInt(360), Op::Sin],
        &[0.0],
        1.0e-5,
        "PLRM3 ch. 8, `sin`, at the argument ISO 32000-2 §7.10.5.3's own DoubleDot \
         example reaches: `{360 mul sin 2 div …}`. 360° is 2π radians, **outside** the \
         only interval WGSL §15.7.4.1 states an accuracy for, and §15.7.4 says \"the \
         accuracy is undefined for input values outside that range\". The corpus keeps \
         the mathematical value and the note; it cannot keep a bound nobody offers.",
    ),
    // ---- cos ----------------------------------------------------------------
    Case::near(
        "cos/zero-degrees",
        about(Table42::Cos),
        &[Op::PushInt(0), Op::Cos],
        &[1.0],
        1.0e-6,
        "PLRM3 ch. 8, `cos`, example `0 cos ⇒ 1.0`.",
    ),
    Case::near(
        "cos/ninety-degrees",
        about(Table42::Cos),
        &[Op::PushInt(90), Op::Cos],
        &[0.0],
        1.0e-6,
        "PLRM3 ch. 8, `cos`, example `90 cos ⇒ 0.0`. The entry prints an exact zero; no \
         binary floating-point conversion of 90° to radians is exactly π/2, so the \
         printed value is the mathematics and the difference is the silence research \
         §1.7 records.",
    ),
    // ---- sqrt ---------------------------------------------------------------
    Case::near(
        "sqrt/four",
        about(Table42::Sqrt),
        &[Op::PushInt(4), Op::Sqrt],
        &[2.0],
        1.0e-6,
        "PLRM3 ch. 8, `sqrt`: \"returns the square root of num\". The entry prints no \
         examples; 4 is chosen because its root is exact in binary32, so the case tests \
         the operator rather than the rounding.",
    ),
    Case::near(
        "sqrt/two",
        about(Table42::Sqrt),
        &[Op::PushInt(2), Op::Sqrt],
        &[1.414_213_6],
        1.0e-6,
        "PLRM3 ch. 8, `sqrt`. WGSL §15.7.4.1 does not require a correctly rounded square \
         root — it is \"Inherited from 1.0 / inverseSqrt(x)\", a 2 ULP reciprocal root \
         through a 2.5 ULP division — where IEEE 754 requires one of a host. This is the \
         cheapest case in the corpus that can differ between the two sides.",
    ),
    Case::error(
        "sqrt/negative-operand",
        about(Table42::Sqrt),
        &[Op::PushInt(-1), Op::Sqrt],
        PsError::RangeCheck,
        "PLRM3 ch. 8, `sqrt`: \"the square root of num, which must be a nonnegative \
         number\"; Errors: `rangecheck`. Stated in the entry's own body, unlike `ln` and \
         `log` below. WGSL's Finite Math Assumption makes an unguarded `sqrt(-1.0)` an \
         *indeterminate value of type f32* — an arbitrary colour that looks like a \
         colour — which is why the guard is not optional.",
    ),
    // ---- exp ----------------------------------------------------------------
    Case::near(
        "exp/fractional-exponent",
        about(Table42::Exp),
        &[Op::PushInt(9), Op::PushReal(0.5), Op::Exp],
        &[3.0],
        1.0e-5,
        "PLRM3 ch. 8, `exp`, example `9 0.5 exp ⇒ 3.0`: \"raises base to the exponent \
         power\".",
    ),
    Case::near(
        "exp/negative-base-integral-exponent",
        about(Table42::Exp),
        &[Op::PushInt(-9), Op::PushInt(-1), Op::Exp],
        &[-0.111_111],
        1.0e-5,
        "PLRM3 ch. 8, `exp`, example `−9 −1 exp ⇒ −0.111111`. A negative base is legal \
         when the exponent has no fractional part, which WGSL's `pow` — \"Inherited from \
         exp2(y * log2(x))\" — cannot express at all, so `exp` has to be built from a \
         sign and parity case split rather than mapped to a builtin.",
    ),
    Case::undefined(
        "exp/negative-base-fractional-exponent",
        about(Table42::Exp),
        &[Op::PushInt(-9), Op::PushReal(0.5), Op::Exp],
        "PLRM3 says only that \"If the exponent has a fractional part, the result is \
         meaningful only if the base is nonnegative\" — which is neither a value nor an \
         error, and ISO 32000-2 adds nothing",
        "PLRM3 ch. 8, `exp`. \"Meaningful only if\" is the whole of it: the entry does \
         not say the result is an error, and does not say what it is. The corpus \
         records the silence.",
    ),
    // ---- ln -----------------------------------------------------------------
    Case::near(
        "ln/ten",
        about(Table42::Ln),
        &[Op::PushInt(10), Op::Ln],
        &[2.302_59],
        1.0e-5,
        "PLRM3 ch. 8, `ln`, example `10 ln ⇒ 2.30259`. The bound is the entry's own \
         printed precision.",
    ),
    Case::near(
        "ln/hundred",
        about(Table42::Ln),
        &[Op::PushInt(100), Op::Ln],
        &[4.605_17],
        1.0e-4,
        "PLRM3 ch. 8, `ln`, example `100 ln ⇒ 4.60517`.",
    ),
    Case::error(
        "ln/zero",
        about(Table42::Ln),
        &[Op::PushInt(0), Op::Ln],
        PsError::RangeCheck,
        "PLRM3 ch. 8, `ln`, Errors: `rangecheck`. **The entry's body does not say which \
         operand raises it** — unlike `sqrt`, which states its restriction. A \
         non-positive operand has no real logarithm and `rangecheck` is the only listed \
         error that can describe it, so the *error name* is the document's and the \
         *identification of this operand* is our reading. Recorded as such in \
         `doc/notes-function-conformance.md`.",
    ),
    Case::error(
        "ln/negative",
        about(Table42::Ln),
        &[Op::PushInt(-1), Op::Ln],
        PsError::RangeCheck,
        "PLRM3 ch. 8, `ln`, as `ln/zero`.",
    ),
    // ---- log ----------------------------------------------------------------
    Case::near(
        "log/ten",
        about(Table42::Log),
        &[Op::PushInt(10), Op::Log],
        &[1.0],
        1.0e-6,
        "PLRM3 ch. 8, `log`, example `10 log ⇒ 1.0`: \"the common logarithm (base 10)\".",
    ),
    Case::near(
        "log/hundred",
        about(Table42::Log),
        &[Op::PushInt(100), Op::Log],
        &[2.0],
        1.0e-6,
        "PLRM3 ch. 8, `log`, example `100 log ⇒ 2.0`. WGSL has no base-10 logarithm, so \
         a lowering multiplies `log2` by a constant — one rounding more than a host \
         `log10`, which many libms compute directly.",
    ),
    Case::error(
        "log/zero",
        about(Table42::Log),
        &[Op::PushInt(0), Op::Log],
        PsError::RangeCheck,
        "PLRM3 ch. 8, `log`, Errors: `rangecheck`, with the same reading — and the same \
         caveat about whose reading it is — as `ln/zero`.",
    ),
];

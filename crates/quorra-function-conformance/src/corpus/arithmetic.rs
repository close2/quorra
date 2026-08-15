//! The fourteen arithmetic operators two conformant implementations must agree on
//! exactly: `abs add sub mul neg div idiv mod ceiling floor round truncate cvi cvr`.
//!
//! "Exactly" is a claim about *these operators*, not about function evaluation: integer
//! arithmetic is pinned on both sides, and the four roundings are "correctly rounded" in
//! WGSL and exact in Rust. `div` is here rather than in [`super::transcendental`]
//! because its *value* is fixed by the clause even though its last bit is not, and every
//! case below is an exact decimal.
//!
//! Four of these are where implementations drift, and each has its own case:
//!
//! - `round` breaks a tie toward the greater value — `−6.5 round ⇒ −6.0` — which is
//!   neither Rust's half-away-from-zero nor WGSL's half-to-even.
//! - `truncate` and `cvi` round toward zero, so `−4.8` gives `−4` where `floor` gives
//!   `−5`.
//! - `add`, `sub` and `mul` change *type* on integer overflow rather than wrapping.
//! - `idiv` and `mod` truncate toward zero and take the dividend's sign.

use crate::case::{Case, PsError, Subject};
use crate::table42::Table42;
use quorra_scene::function::FnOp as Op;

const fn about(operator: Table42) -> Subject {
    Subject::Operator(operator)
}

/// Every case in this family.
pub const CASES: &[Case] = &[
    // ---- abs ----------------------------------------------------------------
    Case::exact(
        "abs/real",
        about(Table42::Abs),
        &[Op::PushReal(4.5), Op::Abs],
        &[4.5],
        "PLRM3 ch. 8, `abs`: \"returns the absolute value of num1\"; its own example \
         `4.5 abs ⇒ 4.5`.",
    ),
    Case::exact(
        "abs/negative-integer",
        about(Table42::Abs),
        &[Op::PushInt(-3), Op::Abs],
        &[3.0],
        "PLRM3 ch. 8, `abs`, example `–3 abs ⇒ 3`. The result is the integer 3, not the \
         real 3.0: \"The type of the result is the same as the type of num1\".",
    ),
    Case::exact(
        "abs/most-negative-integer-becomes-real",
        about(Table42::Abs),
        &[Op::PushInt(i32::MIN), Op::Abs],
        &[2_147_483_648.0],
        "PLRM3 ch. 8, `abs`: the type of the result is the operand's \"unless num1 is \
         the smallest (most negative) integer, in which case the result is a real \
         number\". Rust's `i32::abs` panics here and WGSL's `abs` returns the operand \
         unchanged; the clause says neither.",
    ),
    // ---- add ----------------------------------------------------------------
    Case::exact(
        "add/integers",
        about(Table42::Add),
        &[Op::PushInt(3), Op::PushInt(4), Op::Add],
        &[7.0],
        "PLRM3 ch. 8, `add`, example `3 4 add ⇒ 7`.",
    ),
    Case::near(
        "add/reals",
        about(Table42::Add),
        &[Op::PushReal(9.9), Op::PushReal(1.1), Op::Add],
        &[11.0],
        1.0e-5,
        "PLRM3 ch. 8, `add`, example `9.9 1.1 add ⇒ 11.0`. Neither operand is \
         representable in binary floating point, so the bound is the test's instrument \
         and not a claim: no clause states a precision (research §1.7).",
    ),
    Case::exact(
        "add/integer-overflow-becomes-real",
        about(Table42::Add),
        &[Op::PushInt(i32::MAX), Op::PushInt(1), Op::Add],
        &[2_147_483_648.0],
        "PLRM3 ch. 8, `add`: \"If both operands are integers and the result is within \
         integer range, the result is an integer; otherwise, the result is a real \
         number.\" So an overflow is a change of type, not a wrap and not an error.",
    ),
    // ---- sub ----------------------------------------------------------------
    Case::exact(
        "sub/integers",
        about(Table42::Sub),
        &[Op::PushInt(5), Op::PushInt(3), Op::Sub],
        &[2.0],
        "PLRM3 ch. 8, `sub`: \"returns the result of subtracting num2 from num1\", with \
         the same integer-or-real rule as `add`.",
    ),
    Case::error(
        "sub/mixed-operands-yield-a-real",
        about(Table42::Sub),
        &[Op::PushInt(5), Op::PushReal(3.0), Op::Sub, Op::Not],
        PsError::TypeCheck,
        "PLRM3 ch. 8, `sub`: the result is an integer only \"If both operands are \
         integers\". The type is otherwise invisible in an output, so it is observed \
         through `not`, whose operand rows are `bool₁` and `int₁` only — a real reaching \
         it is a `typecheck`.",
    ),
    // ---- mul ----------------------------------------------------------------
    Case::exact(
        "mul/integers",
        about(Table42::Mul),
        &[Op::PushInt(6), Op::PushInt(7), Op::Mul],
        &[42.0],
        "PLRM3 ch. 8, `mul`: \"returns the product of num1 and num2\".",
    ),
    Case::exact(
        "mul/integer-overflow-becomes-real",
        about(Table42::Mul),
        &[Op::PushInt(65_536), Op::PushInt(65_536), Op::Mul],
        &[4_294_967_296.0],
        "PLRM3 ch. 8, `mul`, same sentence as `add`: outside integer range \"the result \
         is a real number\". The operands are chosen so that the product is 2³², which \
         binary32 holds exactly — a product that needed 32 significant bits would make \
         the expectation a statement about the float format rather than about `mul`.",
    ),
    // ---- neg ----------------------------------------------------------------
    Case::exact(
        "neg/real",
        about(Table42::Neg),
        &[Op::PushReal(4.5), Op::Neg],
        &[-4.5],
        "PLRM3 ch. 8, `neg`, example `4.5 neg ⇒ −4.5`.",
    ),
    Case::exact(
        "neg/most-negative-integer-becomes-real",
        about(Table42::Neg),
        &[Op::PushInt(i32::MIN), Op::Neg],
        &[2_147_483_648.0],
        "PLRM3 ch. 8, `neg`: as `abs`, the result is a real number when the operand is \
         the most negative integer.",
    ),
    // ---- div ----------------------------------------------------------------
    Case::exact(
        "div/example",
        about(Table42::Div),
        &[Op::PushInt(3), Op::PushInt(2), Op::Div],
        &[1.5],
        "PLRM3 ch. 8, `div`, example `3 2 div ⇒ 1.5`.",
    ),
    Case::error(
        "div/result-is-always-real",
        about(Table42::Div),
        &[Op::PushInt(4), Op::PushInt(2), Op::Div, Op::Not],
        PsError::TypeCheck,
        "PLRM3 ch. 8, `div`: \"producing a result that is always a real number even if \
         both operands are integers\", and its example prints `4 2 div ⇒ 2.0`. Observed \
         through `not`, which refuses a real.",
    ),
    Case::error(
        "div/zero-divisor",
        about(Table42::Div),
        &[Op::PushInt(1), Op::PushInt(0), Op::Div],
        PsError::UndefinedResult,
        "PLRM3 ch. 8, `div`, Errors: `undefinedresult`. The entry does not say which \
         operands raise it; a zero divisor is the only case for which the quotient has \
         no value, and ISO 32000-2 §8.7.4.5.2 contemplates it — \"If the function is \
         undefined at any point within the declared domain rectangle, an error may \
         occur\". The identification of the operand is our reading; that no value is \
         defined is the document's.",
    ),
    // ---- idiv ---------------------------------------------------------------
    Case::exact(
        "idiv/truncates-toward-zero",
        about(Table42::Idiv),
        &[Op::PushInt(3), Op::PushInt(2), Op::Idiv],
        &[1.0],
        "PLRM3 ch. 8, `idiv`, example `3 2 idiv ⇒ 1`: \"the integer part of the \
         quotient, with any fractional part discarded\".",
    ),
    Case::exact(
        "idiv/negative-dividend",
        about(Table42::Idiv),
        &[Op::PushInt(-5), Op::PushInt(2), Op::Idiv],
        &[-2.0],
        "PLRM3 ch. 8, `idiv`, example `−5 2 idiv ⇒ −2`. Truncation toward zero, not \
         floor: a floor division would give −3.",
    ),
    Case::error(
        "idiv/real-operand",
        about(Table42::Idiv),
        &[Op::PushReal(5.0), Op::PushInt(2), Op::Idiv],
        PsError::TypeCheck,
        "PLRM3 ch. 8, `idiv`: \"Both operands of idiv must be integers\"; Errors: \
         `typecheck`. A float-only evaluator cannot see this case at all.",
    ),
    Case::error(
        "idiv/zero-divisor",
        about(Table42::Idiv),
        &[Op::PushInt(5), Op::PushInt(0), Op::Idiv],
        PsError::UndefinedResult,
        "PLRM3 ch. 8, `idiv`, Errors: `undefinedresult`. WGSL disagrees loudly: §8.7 \
         makes signed integer division by a runtime zero evaluate to the numerator, \
         which is a value where the clause has none.",
    ),
    Case::undefined(
        "idiv/most-negative-over-minus-one",
        about(Table42::Idiv),
        &[Op::PushInt(i32::MIN), Op::PushInt(-1), Op::Idiv],
        "the quotient 2 147 483 648 is not representable as the 32-bit integer PLRM3 \
         Appendix B describes, and neither ISO 32000-2 nor PLRM3 says what happens: \
         `idiv`'s entry says only that \"the result is an integer\"",
        "PLRM3 ch. 8, `idiv`, and PLRM3 Appendix B Table B.1's 32-bit `integer`. The \
         entry lists `undefinedresult` among its errors but does not say this case \
         raises it, so the corpus records a silence rather than picking one.",
    ),
    // ---- mod ----------------------------------------------------------------
    Case::exact(
        "mod/positive-operands",
        about(Table42::Mod),
        &[Op::PushInt(5), Op::PushInt(3), Op::Mod],
        &[2.0],
        "PLRM3 ch. 8, `mod`, example `5 3 mod ⇒ 2`.",
    ),
    Case::exact(
        "mod/negative-dividend",
        about(Table42::Mod),
        &[Op::PushInt(-5), Op::PushInt(3), Op::Mod],
        &[-2.0],
        "PLRM3 ch. 8, `mod`, example `−5 3 mod ⇒ −2`, with the entry's own gloss: \
         \"The last example above demonstrates that mod is a remainder operation rather \
         than a true modulo operation.\" A true modulo would give 1.",
    ),
    Case::exact(
        "mod/negative-divisor",
        about(Table42::Mod),
        &[Op::PushInt(5), Op::PushInt(-3), Op::Mod],
        &[2.0],
        "PLRM3 ch. 8, `mod`: \"The sign of the result is the same as the sign of the \
         dividend int1.\" The dividend is positive, so the remainder is +2. Derived from \
         the sentence; the entry prints no example with a negative divisor.",
    ),
    Case::error(
        "mod/zero-divisor",
        about(Table42::Mod),
        &[Op::PushInt(5), Op::PushInt(0), Op::Mod],
        PsError::UndefinedResult,
        "PLRM3 ch. 8, `mod`, Errors: `undefinedresult`, as `idiv`.",
    ),
    // ---- ceiling ------------------------------------------------------------
    Case::exact(
        "ceiling/positive",
        about(Table42::Ceiling),
        &[Op::PushReal(3.2), Op::Ceiling],
        &[4.0],
        "PLRM3 ch. 8, `ceiling`, example `3.2 ceiling ⇒ 4.0`.",
    ),
    Case::exact(
        "ceiling/negative",
        about(Table42::Ceiling),
        &[Op::PushReal(-4.8), Op::Ceiling],
        &[-4.0],
        "PLRM3 ch. 8, `ceiling`, example `−4.8 ceiling ⇒ −4.0`: \"the least integer \
         value greater than or equal to num1\".",
    ),
    Case::exact(
        "ceiling/integer-operand-keeps-its-type",
        about(Table42::Ceiling),
        &[Op::PushInt(99), Op::Ceiling],
        &[99.0],
        "PLRM3 ch. 8, `ceiling`, example `99 ceiling ⇒ 99` — not 99.0. \"The type of \
         the result is the same as the type of the operand.\"",
    ),
    // ---- floor --------------------------------------------------------------
    Case::exact(
        "floor/positive",
        about(Table42::Floor),
        &[Op::PushReal(3.2), Op::Floor],
        &[3.0],
        "PLRM3 ch. 8, `floor`, example `3.2 floor ⇒ 3.0`.",
    ),
    Case::exact(
        "floor/negative",
        about(Table42::Floor),
        &[Op::PushReal(-4.8), Op::Floor],
        &[-5.0],
        "PLRM3 ch. 8, `floor`, example `−4.8 floor ⇒ −5.0`. The case that separates \
         `floor` from `truncate`, which gives −4.0 for the same operand.",
    ),
    // ---- truncate -----------------------------------------------------------
    Case::exact(
        "truncate/positive",
        about(Table42::Truncate),
        &[Op::PushReal(3.2), Op::Truncate],
        &[3.0],
        "PLRM3 ch. 8, `truncate`, example `3.2 truncate ⇒ 3.0`.",
    ),
    Case::exact(
        "truncate/negative-rounds-toward-zero",
        about(Table42::Truncate),
        &[Op::PushReal(-4.8), Op::Truncate],
        &[-4.0],
        "PLRM3 ch. 8, `truncate`, example `−4.8 truncate ⇒ −4.0`: \"truncates num1 \
         toward 0 by removing its fractional part\".",
    ),
    Case::exact(
        "truncate/integer-operand-keeps-its-type",
        about(Table42::Truncate),
        &[Op::PushInt(99), Op::Truncate],
        &[99.0],
        "PLRM3 ch. 8, `truncate`, example `99 truncate ⇒ 99`.",
    ),
    // ---- round --------------------------------------------------------------
    Case::exact(
        "round/positive",
        about(Table42::Round),
        &[Op::PushReal(3.2), Op::Round],
        &[3.0],
        "PLRM3 ch. 8, `round`, example `3.2 round ⇒ 3.0`.",
    ),
    Case::exact(
        "round/tie-positive-goes-up",
        about(Table42::Round),
        &[Op::PushReal(6.5), Op::Round],
        &[7.0],
        "PLRM3 ch. 8, `round`, example `6.5 round ⇒ 7.0`. WGSL's `round` is \
         half-to-even and gives 6 here.",
    ),
    Case::exact(
        "round/tie-negative-goes-up",
        about(Table42::Round),
        &[Op::PushReal(-6.5), Op::Round],
        &[-6.0],
        "PLRM3 ch. 8, `round`: \"If num1 is equally close to its two nearest integers, \
         round returns the greater of the two\", with the example `−6.5 round ⇒ −6.0`. \
         **All three of the obvious implementations differ here**: Rust's `f32::round` \
         is half-away-from-zero and gives −7, WGSL's `round` is half-to-even and gives \
         −6 here but 6 for the case above, and only half-toward-greater gives both.",
    ),
    Case::exact(
        "round/negative-not-a-tie",
        about(Table42::Round),
        &[Op::PushReal(-4.8), Op::Round],
        &[-5.0],
        "PLRM3 ch. 8, `round`, example `−4.8 round ⇒ −5.0`.",
    ),
    Case::exact(
        "round/integer-operand-keeps-its-type",
        about(Table42::Round),
        &[Op::PushInt(99), Op::Round],
        &[99.0],
        "PLRM3 ch. 8, `round`, example `99 round ⇒ 99`.",
    ),
    // ---- cvi ----------------------------------------------------------------
    Case::exact(
        "cvi/negative-rounds-toward-zero",
        about(Table42::Cvi),
        &[Op::PushReal(-47.8), Op::Cvi],
        &[-47.0],
        "PLRM3 ch. 8, `cvi`, example `–47.8 cvi ⇒ –47`: \"it truncates any fractional \
         part (that is, rounds it toward 0) and converts it to an integer\".",
    ),
    Case::exact(
        "cvi/positive",
        about(Table42::Cvi),
        &[Op::PushReal(520.9), Op::Cvi],
        &[520.0],
        "PLRM3 ch. 8, `cvi`, example `520.9 cvi ⇒ 520`.",
    ),
    Case::error(
        "cvi/too-large-to-convert",
        about(Table42::Cvi),
        &[Op::PushReal(1.0e20), Op::Cvi],
        PsError::RangeCheck,
        "PLRM3 ch. 8, `cvi`: \"A rangecheck error occurs if a real number is too large \
         to convert to an integer.\" Both host conversions produce a number instead, and \
         not the same one: `1e20f32 as i32` in Rust is 2 147 483 647, and WGSL §15.7.6's \
         clamp gives 2 147 483 520.",
    ),
    // ---- cvr ----------------------------------------------------------------
    Case::exact(
        "cvr/integer-becomes-real",
        about(Table42::Cvr),
        &[Op::PushInt(5), Op::Cvr],
        &[5.0],
        "PLRM3 ch. 8, `cvr`: \"If the operand is an integer, cvr converts it to a real \
         number.\"",
    ),
    Case::error(
        "cvr/result-is-not-an-integer",
        about(Table42::Cvr),
        &[Op::PushInt(5), Op::Cvr, Op::Not],
        PsError::TypeCheck,
        "PLRM3 ch. 8, `cvr` with `not`: after the conversion the value is a real, and \
         `not`'s operand rows are `bool₁` and `int₁`. This is the only way a program can \
         observe that `cvr` did anything at all, which is why the case is a `typecheck` \
         rather than a number.",
    ),
];

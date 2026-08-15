// ISO 32000-2 Table 42, one WGSL function per operator, plus the two clamps §7.10 makes
// normative. A generated §7.10.5 program is a sequence of calls into this file and nothing
// else, so every arithmetic decision the device takes is written down exactly once, here.
//
// Values are `f32` throughout. §7.10.5.1 gives the calculator "expressions involving only
// integers, real numbers, and boolean values"; the integers are carried in `f32` and the
// type of every operand is resolved *before* the shader exists, by `src/function/analyse.rs`
// — which is why `not` appears here twice under two names and no function takes a type tag.
// `true` is 1.0 and `false` is 0.0, which is why `and`, `or` and `xor` need only their
// bitwise reading: with those two values the logical reading agrees.
//
// # Two things this file does differently from the spike that preceded it, on purpose
//
// - `ps_round` is PLRM3's half-toward-greater, not Rust's half-away-from-zero and not
//   WGSL's half-to-even. All three differ at `-6.5`, and PLRM3's own entry prints
//   `-6.5 round => -6.0` as its example.
// - `ps_exp` is not `pow`. PLRM3's entry gives `-9 -1 exp => -0.111111`, a negative base;
//   WGSL's `pow` is "inherited from exp2(y * log2(x))" and undefined there.
//
// # Where the specification says "error", and what happens here instead
//
// PLRM3 makes a zero divisor `undefinedresult`, a negative `sqrt` operand `rangecheck`, and
// several more; ISO 32000-2 §8.7.4.5.2 says of a function undefined inside its declared
// domain that "an error may occur". Neither document defines a substitute value, and a
// fragment shader has nowhere to raise to.
//
// **Every one of those is guarded here, and the guard yields 0.0.** The guard is not
// optional: WGSL §15.7.2's Finite Math Assumption says that an expression which overflows
// or yields a NaN produces "an indeterminate value of the target type" — an arbitrary
// colour that looks like a colour, which CLAUDE.md principle 6 prices as the worst outcome
// there is. The *value* 0.0 matches the caller's own evaluator and is a decision neither
// side has written into a contract yet; `doc/notes-function-generator.md` records it as
// open. Every guard below is an `if` rather than a `select` so that the undefined operation
// is not evaluated at all.

// ISO 32000-2 §7.10, and it is the reason this function exists rather than a bare `clamp`:
//
//   "Input values passed to the function shall be clipped to the domain, and output values
//    produced by the function shall be clipped to the range."
//
// INVARIANT: `low <= high`. WGSL's `clamp` is indeterminate otherwise, and
// `function::analyse` refuses a Domain or a Range that is not an interval before any shader
// is generated. Nothing here re-checks it, because a check that cannot fail teaches a
// reader the wrong thing about who owns the guarantee.
fn ps_clip(value: f32, low: f32, high: f32) -> f32 {
    return clamp(value, low, high);
}

// The value a pop from an empty operand stack yields. ISO 32000-2 defines nothing here and
// PostScript would raise `stackunderflow`; this is a deliberate choice, counted by
// `Analysis::empty_stack_pops` so a frame can report it rather than adopt it invisibly.
const PS_EMPTY_STACK: f32 = 0.0;

const PS_DEGREES_PER_RADIAN: f32 = 57.29577951308232;
const PS_RADIANS_PER_DEGREE: f32 = 0.017453292519943295;
// log10(2), for the one multiply that turns WGSL's `log2` into Table 42's `log`.
const PS_LOG10_OF_2: f32 = 0.30102999566398120;

// The largest f32 strictly below 2^31 and the exact value of -2^31 — the interval WGSL
// §15.7.6's float-to-integer conversion clamps to, whose own worked example is
// "1e20f converted to i32 is the maximum i32 value, 2147483520i". The clamp is written out
// rather than left to the conversion so that the reference evaluator in `tests/` can hold
// the same bound: a host `as i32` saturates to 2147483647 instead, and the two would
// disagree for every operand past 2^31.
const PS_INT_MAX: f32 = 2147483520.0;
const PS_INT_MIN: f32 = -2147483648.0;

fn ps_to_int(a: f32) -> i32 {
    return i32(clamp(trunc(a), PS_INT_MIN, PS_INT_MAX));
}

// Table 42: abs — absolute value.
fn ps_abs(a: f32) -> f32 { return abs(a); }

// Table 42: add — sum.
fn ps_add(a: f32, b: f32) -> f32 { return a + b; }

// Table 42: atan — `num den atan`, the angle in degrees in [0, 360).
//
// PLRM3: "Either num or den may be 0, but not both"; both zero is `undefinedresult`.
// WGSL §15.7.4.1 gives `atan2` 4096 ULP, which after the conversion below is 0.028 degrees
// — the worst row in the accuracy table by three orders of magnitude, and the reason
// `Binary::is_inexact` names this operator.
fn ps_atan(num: f32, den: f32) -> f32 {
    if (num == 0.0 && den == 0.0) { return 0.0; }
    let degrees = atan2(num, den) * PS_DEGREES_PER_RADIAN;
    if (degrees < 0.0) { return degrees + 360.0; }
    return degrees;
}

// Table 42: ceiling — the least integer not less than the operand.
fn ps_ceiling(a: f32) -> f32 { return ceil(a); }

// Reduce an angle in degrees to [-180, 180), which is [-pi, pi) in radians.
//
// This is not tidiness. WGSL §15.7.4.1 bounds `sin` and `cos` by "absolute error at most
// 2^-11 when x is in the interval [-pi, pi]", and §15.7.4 adds that "the accuracy is
// undefined for input values outside that range" — while §7.10.5.3's own worked example,
// the `DoubleDot` spot function, evaluates `sin` at +-360 degrees. Without this, that
// example runs entirely outside the only guarantee WGSL offers.
//
// What it costs, stated: the reduction is one subtraction of a product, so for an operand of
// large magnitude the reduced angle carries the operand's own absolute error. A program
// that takes the sine of a million degrees gets an answer whose accuracy this cannot rescue,
// and no cheap reduction can.
fn ps_reduce_degrees(a: f32) -> f32 {
    return a - 360.0 * floor((a + 180.0) / 360.0);
}

// Table 42: cos — cosine of an angle in degrees.
fn ps_cos(a: f32) -> f32 { return cos(ps_reduce_degrees(a) * PS_RADIANS_PER_DEGREE); }

// Table 42: cvi — convert to integer, truncating toward zero.
fn ps_cvi(a: f32) -> f32 { return trunc(a); }

// Table 42: cvr — convert to real. A no-op in a representation that is already `f32`.
fn ps_cvr(a: f32) -> f32 { return a; }

// Table 42: div — real division. PLRM3 makes a zero divisor `undefinedresult`.
//
// WGSL §15.7.4.1 allows `x / y` 2.5 ULP where IEEE 754 requires the host's `/` to be
// correctly rounded, which is why `Binary::is_inexact` names this operator even though the
// caller did not.
fn ps_div(a: f32, b: f32) -> f32 {
    if (b == 0.0) { return 0.0; }
    return a / b;
}

// Table 42: exp — `base exponent exp`.
//
// PLRM3: "raises base to the exponent power ... If the exponent has a fractional part, the
// result is meaningful only if the base is nonnegative", and the entry's example is
// `-9 -1 exp => -0.111111`. So a negative base with an integer exponent is *defined* and
// carries the sign of that exponent's parity, which `pow` cannot express.
fn ps_exp(base: f32, exponent: f32) -> f32 {
    let magnitude = abs(base);
    if (magnitude == 0.0) {
        // 0^0 is 1; 0^p for positive p is 0; 0^p for negative p is a division by zero,
        // which is guarded to 0 like every other domain exit here.
        if (exponent == 0.0) { return 1.0; }
        return 0.0;
    }
    let size = exp2(exponent * log2(magnitude));
    if (base > 0.0) { return size; }
    if (exponent != trunc(exponent)) { return 0.0; }
    // `fract(|e| / 2)` is non-zero exactly when the integer `e` is odd, and is exact for
    // every |e| below 2^23 — past which `e` has no odd values left to distinguish.
    if (fract(abs(exponent) * 0.5) != 0.0) { return -size; }
    return size;
}

// Table 42: floor — the greatest integer not greater than the operand.
fn ps_floor(a: f32) -> f32 { return floor(a); }

// Table 42: idiv — integer division, truncating. PLRM3: "Both operands of idiv must be
// integers", which `function::analyse` enforces statically; a zero divisor is
// `undefinedresult`.
fn ps_idiv(a: f32, b: f32) -> f32 {
    let divisor = ps_to_int(b);
    if (divisor == 0) { return 0.0; }
    return f32(ps_to_int(a) / divisor);
}

// Table 42: ln — natural logarithm. PLRM3 makes a non-positive operand `rangecheck`.
fn ps_ln(a: f32) -> f32 {
    if (a <= 0.0) { return 0.0; }
    return log(a);
}

// Table 42: log — common (base-10) logarithm, and one multiply more rounding than a host
// `log10` that computes it directly.
fn ps_log(a: f32) -> f32 {
    if (a <= 0.0) { return 0.0; }
    return log2(a) * PS_LOG10_OF_2;
}

// Table 42: mod — integer remainder. PLRM3: the "sign of the result is the same as the sign
// of the dividend", which is what both WGSL's and Rust's `%` do on integers.
fn ps_mod(a: f32, b: f32) -> f32 {
    let divisor = ps_to_int(b);
    if (divisor == 0) { return 0.0; }
    return f32(ps_to_int(a) % divisor);
}

// Table 42: mul — product.
fn ps_mul(a: f32, b: f32) -> f32 { return a * b; }

// Table 42: neg — negation.
fn ps_neg(a: f32) -> f32 { return -a; }

// Table 42: round — the nearest integer.
//
// PLRM3, verbatim: "returns the integer value nearest to num1. If num1 is equally close to
// its two nearest integers, round returns the greater of the two." Its examples include
// `-6.5 round => -6.0`.
//
// **Neither built-in is this rule.** WGSL's `round` is half-to-even ("the result is k when k
// is even, and k + 1 when k is odd"); Rust's `f32::round` is half-away-from-zero, so
// `(-6.5f32).round()` is -7. All three answers differ at -6.5, so both sides spell the rule
// out. Written as floor-then-step rather than `floor(a + 0.5)` because the addition would
// round for operands near 2^23 and hand back the wrong integer; for those operands
// `floor(a)` is `a` and the difference below is 0, which is correct.
fn ps_round(a: f32) -> f32 {
    let below = floor(a);
    if ((a - below) >= 0.5) { return below + 1.0; }
    return below;
}

// Table 42: sin — sine of an angle in degrees. See `ps_reduce_degrees`.
fn ps_sin(a: f32) -> f32 { return sin(ps_reduce_degrees(a) * PS_RADIANS_PER_DEGREE); }

// Table 42: sqrt — square root. PLRM3: "the square root of num, which must be a nonnegative
// number"; a negative operand is `rangecheck`.
//
// WGSL §15.7.4.1 defines `sqrt` as "inherited from 1.0 / inverseSqrt(x)" — a 2 ULP
// reciprocal square root followed by a 2.5 ULP division — where IEEE 754 requires the host's
// to be correctly rounded. Hence `Unary::is_inexact`.
fn ps_sqrt(a: f32) -> f32 {
    if (a < 0.0) { return 0.0; }
    return sqrt(a);
}

// Table 42: sub — difference.
fn ps_sub(a: f32, b: f32) -> f32 { return a - b; }

// Table 42: truncate — the operand with its fractional part removed.
fn ps_truncate(a: f32) -> f32 { return trunc(a); }

// Table 42: and, or, xor — bitwise on integers, and with `true` as 1 and `false` as 0 the
// logical reading of each agrees with the bitwise one, so one lowering serves both.
// `function::analyse` refuses a real operand rather than truncating it here.
fn ps_and(a: f32, b: f32) -> f32 { return f32(ps_to_int(a) & ps_to_int(b)); }
fn ps_or(a: f32, b: f32) -> f32 { return f32(ps_to_int(a) | ps_to_int(b)); }
fn ps_xor(a: f32, b: f32) -> f32 { return f32(ps_to_int(a) ^ ps_to_int(b)); }

// Table 42: bitshift — `int shift bitshift`, left for a positive shift.
//
// PLRM3: "shifts the binary representation of int1 left by shift bits and returns the
// result. Bits shifted out are lost; bits shifted in are 0. If shift is negative, a right
// shift by -shift bits is performed." **The right shift is therefore a logical one**, which
// is why the negative arm reinterprets through `u32` rather than using i32's `>>` — that
// operator sign-extends, and would shift ones in for a negative operand.
//
// A shift of 32 or more clears the value. PLRM3's entry does not state what happens past
// the operand's width and WGSL takes the shift amount modulo the bit width, which would
// make `1 32 bitshift` yield 1; that is a reading nothing in either document supports, so
// the width is handled explicitly here.
fn ps_bitshift(value: f32, shift: f32) -> f32 {
    let bits = ps_to_int(value);
    let by = ps_to_int(shift);
    if (by >= 32 || by <= -32) { return 0.0; }
    if (by >= 0) { return f32(bits << u32(by)); }
    return f32(bitcast<i32>(bitcast<u32>(bits) >> u32(-by)));
}

// Table 42: eq, ne, ge, gt, le, lt.
//
// Exact equality, which is what PLRM3's `eq` entry describes: "Simple objects are equal if
// their types and values are the same", with an integer and a real of the same mathematical
// value equal. The caller's own evaluator compares within an absolute `f32::EPSILON`
// instead; that difference is recorded in `doc/notes-function-generator.md` rather than
// copied here, because matching another implementation is what CLAUDE.md principle 5
// forbids outright.
//
// WGSL §15.7.4.1 gives all six "correct result", so these are exact on both sides — and
// they are the principal amplifiers: each turns a last-bit disagreement upstream into a
// different colour.
fn ps_eq(a: f32, b: f32) -> f32 { return f32(a == b); }
fn ps_ne(a: f32, b: f32) -> f32 { return f32(a != b); }
fn ps_ge(a: f32, b: f32) -> f32 { return f32(a >= b); }
fn ps_gt(a: f32, b: f32) -> f32 { return f32(a > b); }
fn ps_le(a: f32, b: f32) -> f32 { return f32(a <= b); }
fn ps_lt(a: f32, b: f32) -> f32 { return f32(a < b); }

// Table 42: not — the two readings, chosen before the frame by `function::analyse`.
//
// Logical negation over a boolean, one's complement over an integer. The caller's compiled
// form carries no type on a literal and their evaluator implements the logical one only, so
// `63 not` yields 0.0 there where PLRM3 says -64. That is what `FnOp::PushInt` exists to fix,
// and this is where the fix is spent: neither reading costs anything at run time because the
// choice is already made.
fn ps_not(a: f32) -> f32 { return f32(a == 0.0); }
fn ps_bit_not(a: f32) -> f32 { return f32(~ps_to_int(a)); }

//! Table 42's 21 arithmetic operators, from PLRM3's entries.
//!
//! Every rule below is quoted in the function that implements it. Four of them are the
//! reason this module exists at all, because no host built-in has them:
//!
//! - `round` breaks a tie **toward the greater** value, so `−6.5 round ⇒ −6.0`. Rust's
//!   `f32::round` is half-away-from-zero and gives −7; WGSL's `round` is half-to-even
//!   and gives −6 here but 6 for `6.5`, where PLRM3 also says 7.
//! - `ceiling`, `floor`, `round` and `truncate` **keep the operand's type**, so `99
//!   round` is the integer 99 and not the real 99.0.
//! - `add`, `sub` and `mul` return an integer only if both operands were integers *and*
//!   the result fits; otherwise a real. So integer overflow is not wrapping and not an
//!   error — it is a change of type.
//! - `div` is always real, even for `4 2 div`.

use super::error::EvalError;
use super::value::Value;
use crate::case::PsError;
use quorra_scene::function::FnOp;

/// How many operands an arithmetic operator takes, or `None` if the operator is not
/// one of Table 42's arithmetic 21.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity {
    /// `num op result`.
    Unary,
    /// `num₁ num₂ op result`.
    Binary,
}

/// The arity of an arithmetic operator, or `None` for anything else.
#[must_use]
pub const fn arity(op: FnOp) -> Option<Arity> {
    match op {
        FnOp::Abs
        | FnOp::Ceiling
        | FnOp::Cos
        | FnOp::Cvi
        | FnOp::Cvr
        | FnOp::Floor
        | FnOp::Ln
        | FnOp::Log
        | FnOp::Neg
        | FnOp::Round
        | FnOp::Sin
        | FnOp::Sqrt
        | FnOp::Truncate => Some(Arity::Unary),
        FnOp::Add
        | FnOp::Atan
        | FnOp::Div
        | FnOp::Exp
        | FnOp::Idiv
        | FnOp::Mod
        | FnOp::Mul
        | FnOp::Sub => Some(Arity::Binary),
        _ => None,
    }
}

/// Apply a unary arithmetic operator.
///
/// # Errors
///
/// Whatever the operator's PLRM3 entry names, and [`EvalError::Undefined`] where it
/// names nothing.
pub fn unary(op: FnOp, a: Value) -> Result<Value, EvalError> {
    match op {
        // "returns the absolute value of num1. The type of the result is the same as
        // the type of num1 unless num1 is the smallest (most negative) integer, in
        // which case the result is a real number."
        FnOp::Abs => integer_preserving(a, i32::checked_abs, f32::abs),
        // "returns the negative of num1. The type of the result is the same as the type
        // of num1 unless num1 is the smallest (most negative) integer, in which case
        // the result is a real number."
        FnOp::Neg => integer_preserving(a, i32::checked_neg, |r| -r),
        // "returns the least integer value greater than or equal to num1. The type of
        // the result is the same as the type of the operand."
        FnOp::Ceiling => type_preserving_rounding(a, f32::ceil),
        // "returns the greatest integer value less than or equal to num1."
        FnOp::Floor => type_preserving_rounding(a, f32::floor),
        // "truncates num1 toward 0 by removing its fractional part."
        FnOp::Truncate => type_preserving_rounding(a, f32::trunc),
        // "returns the integer value nearest to num1. If num1 is equally close to its
        // two nearest integers, round returns the greater of the two."
        FnOp::Round => type_preserving_rounding(a, round_half_toward_greater),
        FnOp::Cvi => convert_to_integer(a),
        // "(convert to real) takes an integer, real, or string object and produces a
        // real result."
        FnOp::Cvr => Ok(Value::Real(a.real()?)),
        // "returns the sine of angle, which is interpreted as an angle in degrees."
        FnOp::Sin => Ok(Value::Real(a.real()?.to_radians().sin())),
        // "returns the cosine of angle, which is interpreted as an angle in degrees."
        FnOp::Cos => Ok(Value::Real(a.real()?.to_radians().cos())),
        FnOp::Sqrt => square_root(a),
        // "returns the natural logarithm (base e) of num."
        FnOp::Ln => logarithm(a, f32::ln),
        // "returns the common logarithm (base 10) of num."
        FnOp::Log => logarithm(a, f32::log10),
        _ => Err(EvalError::Malformed("not a unary arithmetic operator")),
    }
}

/// Apply a binary arithmetic operator.
///
/// # Errors
///
/// As [`unary`].
pub fn binary(op: FnOp, a: Value, b: Value) -> Result<Value, EvalError> {
    match op {
        FnOp::Add => integer_or_real(a, b, i32::checked_add, |x, y| x + y),
        FnOp::Sub => integer_or_real(a, b, i32::checked_sub, |x, y| x - y),
        FnOp::Mul => integer_or_real(a, b, i32::checked_mul, |x, y| x * y),
        FnOp::Div => divide(a, b),
        FnOp::Idiv => integer_divide(a, b),
        FnOp::Mod => remainder(a, b),
        FnOp::Atan => arc_tangent(a, b),
        FnOp::Exp => power(a, b),
        _ => Err(EvalError::Malformed("not a binary arithmetic operator")),
    }
}

/// PLRM3's `round`, which is neither of the two roundings a host provides.
///
/// > returns the integer value nearest to num1. If num1 is equally close to its two
/// > nearest integers, round returns the greater of the two.
///
/// Written from the floor and the fraction rather than as `(x + 0.5).floor()`: adding
/// 0.5 to a value at the top of `f32`'s integral range rounds *before* the floor sees
/// it, which would turn 8 388 609 into 8 388 610.
fn round_half_toward_greater(value: f32) -> f32 {
    let below = value.floor();
    if value - below >= 0.5 {
        below + 1.0
    } else {
        below
    }
}

/// `abs` and `neg`: the result keeps the operand's type, except at `i32::MIN`, where
/// PLRM3 says it becomes a real.
fn integer_preserving(
    a: Value,
    on_integer: fn(i32) -> Option<i32>,
    on_real: fn(f32) -> f32,
) -> Result<Value, EvalError> {
    match a {
        Value::Int(i) => match on_integer(i) {
            Some(result) => Ok(Value::Int(result)),
            // The only operand for which either operation overflows is `i32::MIN`, and
            // PLRM3 names the answer: the result is a real number.
            None => Ok(Value::Real(2_147_483_648.0)),
        },
        Value::Real(r) => Ok(Value::Real(on_real(r))),
        Value::Bool(_) => Err(PsError::TypeCheck.into()),
    }
}

/// `ceiling`, `floor`, `round`, `truncate`: "The type of the result is the same as the
/// type of the operand", so an integer operand is returned untouched.
fn type_preserving_rounding(a: Value, on_real: fn(f32) -> f32) -> Result<Value, EvalError> {
    match a {
        Value::Int(i) => Ok(Value::Int(i)),
        Value::Real(r) => Ok(Value::Real(on_real(r))),
        Value::Bool(_) => Err(PsError::TypeCheck.into()),
    }
}

/// `cvi`: "If the operand is a real number, it truncates any fractional part (that is,
/// rounds it toward 0) and converts it to an integer. […] A rangecheck error occurs if
/// a real number is too large to convert to an integer."
fn convert_to_integer(a: Value) -> Result<Value, EvalError> {
    match a {
        Value::Int(i) => Ok(Value::Int(i)),
        Value::Real(r) => {
            let truncated = r.trunc();
            if !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&f64::from(truncated)) {
                return Err(PsError::RangeCheck.into());
            }
            // In range by the test above, so the cast is the truncation the clause
            // describes and not a wrap.
            #[allow(clippy::cast_possible_truncation)]
            Ok(Value::Int(truncated as i32))
        }
        Value::Bool(_) => Err(PsError::TypeCheck.into()),
    }
}

/// `sqrt`: "returns the square root of num, which must be a nonnegative number." The
/// entry states the restriction and lists `rangecheck`, so this one is not an
/// inference.
fn square_root(a: Value) -> Result<Value, EvalError> {
    let value = a.real()?;
    if value < 0.0 {
        return Err(PsError::RangeCheck.into());
    }
    Ok(Value::Real(value.sqrt()))
}

/// `ln` and `log`. Their entries state the function and list `rangecheck` without
/// saying which operand raises it; a non-positive operand has no real logarithm and
/// `rangecheck` is the only listed error that can describe it. **That last step is our
/// reading, not the document's**, and it is recorded as such in
/// `doc/notes-function-conformance.md`.
fn logarithm(a: Value, on_real: fn(f32) -> f32) -> Result<Value, EvalError> {
    let value = a.real()?;
    if value <= 0.0 {
        return Err(PsError::RangeCheck.into());
    }
    Ok(Value::Real(on_real(value)))
}

/// `add`, `sub`, `mul`: "If both operands are integers and the result is within integer
/// range, the result is an integer; otherwise, the result is a real number."
fn integer_or_real(
    a: Value,
    b: Value,
    on_integers: fn(i32, i32) -> Option<i32>,
    on_reals: fn(f32, f32) -> f32,
) -> Result<Value, EvalError> {
    if let (Value::Int(x), Value::Int(y)) = (a, b)
        && let Some(result) = on_integers(x, y)
    {
        return Ok(Value::Int(result));
    }
    Ok(Value::Real(on_reals(a.real()?, b.real()?)))
}

/// `div`: "divides num1 by num2, producing a result that is always a real number even
/// if both operands are integers." A zero divisor has no result; `undefinedresult` is
/// the error the entry lists.
fn divide(a: Value, b: Value) -> Result<Value, EvalError> {
    let divisor = b.real()?;
    if divisor == 0.0 {
        return Err(PsError::UndefinedResult.into());
    }
    Ok(Value::Real(a.real()? / divisor))
}

/// `idiv`: "divides int1 by int2 and returns the integer part of the quotient, with any
/// fractional part discarded. Both operands of idiv must be integers and the result is
/// an integer."
fn integer_divide(a: Value, b: Value) -> Result<Value, EvalError> {
    let (dividend, divisor) = (a.integer()?, b.integer()?);
    if divisor == 0 {
        return Err(PsError::UndefinedResult.into());
    }
    dividend
        .checked_div(divisor)
        .map(Value::Int)
        .ok_or(EvalError::Undefined(
            "idiv of i32::MIN by -1: the quotient is not an integer PLRM3's 32-bit \
             `integer` can hold, and neither document says what happens",
        ))
}

/// `mod`: "returns the remainder that results from dividing int1 by int2. The sign of
/// the result is the same as the sign of the dividend int1."
///
/// `i32::MIN mod -1` is 0, which is representable, so unlike `idiv` there is nothing
/// undefined about it — `wrapping_rem` is exact here rather than a wrap.
fn remainder(a: Value, b: Value) -> Result<Value, EvalError> {
    let (dividend, divisor) = (a.integer()?, b.integer()?);
    if divisor == 0 {
        return Err(PsError::UndefinedResult.into());
    }
    // The divisor is non-zero by the test above, and `i32::MIN` remainder −1 is 0,
    // which is representable — so this wraps nothing and cannot trap.
    #[allow(clippy::arithmetic_side_effects)]
    Ok(Value::Int(dividend.wrapping_rem(divisor)))
}

/// `atan`: "returns the angle (in degrees between 0 and 360) whose tangent is num
/// divided by den. Either num or den may be 0, but not both."
///
/// The quadrant rule is the entry's: "a positive num yields a result in the positive y
/// plane, while a positive den yields a result in the positive x plane" — which is a
/// two-argument arc tangent, with the result folded into `0..360` rather than
/// `-180..180`. The entry's own examples pin it: `−100 0 atan ⇒ 270.0`.
fn arc_tangent(a: Value, b: Value) -> Result<Value, EvalError> {
    let (numerator, denominator) = (a.real()?, b.real()?);
    if numerator == 0.0 && denominator == 0.0 {
        return Err(PsError::UndefinedResult.into());
    }
    let degrees = numerator.atan2(denominator).to_degrees();
    Ok(Value::Real(if degrees < 0.0 {
        degrees + 360.0
    } else {
        degrees
    }))
}

/// `exp`: "raises base to the exponent power. […] If the exponent has a fractional
/// part, the result is meaningful only if the base is nonnegative. The result is always
/// a real number."
///
/// "Meaningful only if" is not a value and not an error: the entry declines to say what
/// a fractional power of a negative base produces, so this declines too.
fn power(a: Value, b: Value) -> Result<Value, EvalError> {
    let (base, exponent) = (a.real()?, b.real()?);
    if base < 0.0 && exponent.fract() != 0.0 {
        return Err(EvalError::Undefined(
            "exp with a negative base and a fractional exponent: PLRM3 says the result \
             is 'meaningful only if the base is nonnegative' and names neither a value \
             nor an error",
        ));
    }
    Ok(Value::Real(base.powf(exponent)))
}

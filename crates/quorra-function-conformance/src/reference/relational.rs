//! Table 42's relational, boolean and bitwise operators, from PLRM3's entries.
//!
//! ISO 32000-2 Annex B writes four of them with two operand rows —
//! `bool₁|int₁ bool₂|int₂ and bool₃|int₃` — and that bar is the subject of this module.
//! `and`, `or` and `xor` mean the same thing either way once `true` is 1 and `false` is
//! 0, which is why an implementation that ignores the distinction gets them right; `not`
//! does not, and an evaluator that treats every value as a number answers `63 not` with
//! 0 where the standard says −64.

use super::error::EvalError;
use super::value::Value;
use crate::case::PsError;
use quorra_scene::function::FnOp;

/// Whether `op` is one of the ten that take two operands. `not` is the eleventh and is
/// unary.
#[must_use]
pub const fn is_binary(op: FnOp) -> bool {
    matches!(
        op,
        FnOp::And
            | FnOp::Bitshift
            | FnOp::Eq
            | FnOp::Ge
            | FnOp::Gt
            | FnOp::Le
            | FnOp::Lt
            | FnOp::Ne
            | FnOp::Or
            | FnOp::Xor
    )
}

/// Apply a relational, boolean or bitwise operator of two operands.
///
/// # Errors
///
/// Whatever the operator's PLRM3 entry names, and [`EvalError::Undefined`] for a
/// `bitshift` whose count is at least the width of the operand.
pub fn binary(op: FnOp, a: Value, b: Value) -> Result<Value, EvalError> {
    match op {
        // "pops two objects from the operand stack and pushes true if they are equal".
        FnOp::Eq => equal(a, b).map(Value::Bool),
        // "pushes false if they are equal, or true if not."
        FnOp::Ne => equal(a, b).map(|same| Value::Bool(!same)),
        FnOp::Gt => compare(a, b, |x, y| x > y),
        FnOp::Ge => compare(a, b, |x, y| x >= y),
        FnOp::Lt => compare(a, b, |x, y| x < y),
        FnOp::Le => compare(a, b, |x, y| x <= y),
        // "returns the logical conjunction of the operands if they are boolean. If the
        // operands are integers, and returns the bitwise 'and' of their binary
        // representations."
        FnOp::And => logical_or_bitwise(a, b, |x, y| x && y, |x, y| x & y),
        FnOp::Or => logical_or_bitwise(a, b, |x, y| x || y, |x, y| x | y),
        FnOp::Xor => logical_or_bitwise(a, b, |x, y| x != y, |x, y| x ^ y),
        FnOp::Bitshift => shift(a, b),
        _ => Err(EvalError::Malformed(
            "not a binary relational, boolean or bitwise operator",
        )),
    }
}

/// `not`, which is unary and is two operators.
///
/// > returns the logical negation of the operand if it is boolean. If the operand is an
/// > integer, not returns the bitwise complement (ones complement) of its binary
/// > representation.
///
/// # Errors
///
/// `typecheck` for a real: the entry's two operand forms are `bool₁` and `int₁`, and
/// nothing else.
pub fn not(a: Value) -> Result<Value, EvalError> {
    match a {
        Value::Bool(b) => Ok(Value::Bool(!b)),
        Value::Int(i) => Ok(Value::Int(!i)),
        Value::Real(_) => Err(PsError::TypeCheck.into()),
    }
}

/// PLRM3's `eq`, which is equality of type *and* value with one coercion:
///
/// > Simple objects are equal if their types and values are the same. […] This operator
/// > performs some type conversions. Integers and real numbers can be compared freely:
/// > an integer and a real number representing the same mathematical value are
/// > considered equal by **eq**.
///
/// So a boolean is never equal to a number — the types differ and no conversion is
/// offered between them — and `1 1.0 eq` is true. There is no tolerance anywhere in the
/// entry; an epsilon comparison is a different operator.
// The exact comparison *is* the operator; see the paragraph above.
#[allow(clippy::float_cmp)]
fn equal(a: Value, b: Value) -> Result<bool, EvalError> {
    Ok(match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Bool(_), _) | (_, Value::Bool(_)) => false,
        _ => a.number()? == b.number()?,
    })
}

/// `gt`, `ge`, `lt`, `le`: "If both operands are numbers, [the operator] compares their
/// mathematical values. […] If the operands are of other types … a typecheck error
/// occurs."
fn compare(a: Value, b: Value, test: fn(f64, f64) -> bool) -> Result<Value, EvalError> {
    Ok(Value::Bool(test(a.number()?, b.number()?)))
}

/// `and`, `or`, `xor`: logical on two booleans, bitwise on two integers, `typecheck` on
/// anything else — including one of each, which no operand row admits.
fn logical_or_bitwise(
    a: Value,
    b: Value,
    on_booleans: fn(bool, bool) -> bool,
    on_integers: fn(i32, i32) -> i32,
) -> Result<Value, EvalError> {
    match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => Ok(Value::Bool(on_booleans(x, y))),
        (Value::Int(x), Value::Int(y)) => Ok(Value::Int(on_integers(x, y))),
        _ => Err(PsError::TypeCheck.into()),
    }
}

/// `bitshift`:
///
/// > shifts the binary representation of int1 left by shift bits and returns the result.
/// > Bits shifted out are lost; bits shifted in are 0. If shift is negative, a right
/// > shift by –shift bits is performed. This operation produces an arithmetically
/// > correct result only for positive values of int1. Both int1 and shift must be
/// > integers.
///
/// "Bits shifted in are 0" is the load-bearing sentence and it makes the right shift a
/// **logical** one, which the next sentence confirms by warning that the result is
/// arithmetically correct only for a positive operand — a warning an arithmetic shift
/// would not need. Rust's `>>` on `i32` and WGSL's `>>` on `i32` are both arithmetic, so
/// both give −4 for `−8 −1 bitshift` where this gives 2 147 483 644.
///
/// A shift of 32 or more is where the entry stops: it says bits shifted out are lost,
/// which suggests 0, but it does not say so, and WGSL takes the count modulo the width
/// instead, which would give the operand back. That is a silence, and it is returned as
/// one.
fn shift(a: Value, b: Value) -> Result<Value, EvalError> {
    let (value, count) = (a.integer()?, b.integer()?);
    let Some(magnitude) = count.checked_abs().filter(|c| *c < 32) else {
        return Err(EvalError::Undefined(
            "bitshift by 32 or more places: PLRM3's entry does not say what a shift at \
             or past the operand's width produces, and WGSL's shift takes the count \
             modulo the width rather than saturating",
        ));
    };
    // The clause shifts "the binary representation", not the arithmetic value, so the
    // shifts happen on the unsigned view and the result is reinterpreted. Both casts
    // are reinterpretations of the same 32 bits, which is exactly what is asked for.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
    let shifted = if count >= 0 {
        ((value as u32) << magnitude) as i32
    } else {
        ((value as u32) >> magnitude) as i32
    };
    Ok(Value::Int(shifted))
}

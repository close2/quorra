#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::arithmetic_side_effects,
    reason = "an evaluator of Table 42 is float comparison and saturating integer \
              conversion by definition; every cast here is the one `function_ops.wgsl` \
              performs, written out so the two sides hold the same bound"
)]

//! An independent evaluation of a §7.10.5 program on the processor. **Tests only.**
//!
//! ADR 0037's precedent, stated there and applied here: *an independent implementation a test
//! compares is stronger evidence than agreement by construction.* This is a plain stack
//! machine over `&[FnOp]` with a program counter and a `Vec` — it shares no code, no table and
//! no data structure with `function::analyse`'s compile-time walk or with
//! `function::generate`'s output, so a disagreement between them is a defect in one of the two
//! rather than a tautology.
//!
//! The one structural difference is the point of the comparison: **this machine carries the
//! type of every value at run time, where the analyser proves it at compile time.** Table 42's
//! `not` means two different operations depending on it, so a machine that does not track the
//! type cannot implement `not` at all — which is exactly the defect the spike found in the
//! caller's own evaluator, where `63 not` yields `0.0` where PLRM3 says `-64`.
//!
//! Every operator below is written from what PLRM3 and ISO 32000-2 say it does, and the tests
//! that pin the awkward ones (`round`, `exp`, `bitshift`, `atan`) assert values quoted from
//! those documents rather than values observed from the shader. That is the direction
//! principle 5 requires: where the two disagree, the clause decides.

use quorra_gpu::function::range_bounds;
use quorra_scene::{FnOp, FnRange};

use super::programs::Witness;

/// The interval WGSL §15.7.6's float-to-integer conversion clamps to, which
/// `function_ops.wgsl` writes out explicitly so that both sides hold the same bound. A host
/// `as i32` saturates to 2 147 483 647 instead, and the two would disagree past 2³¹.
pub const INT_MAX: f32 = 2_147_483_520.0;
/// The other end of that interval, which is exactly −2³¹.
pub const INT_MIN: f32 = -2_147_483_648.0;

/// Table 42's float-to-integer conversion, as `ps_to_int` performs it.
#[must_use]
pub fn to_int(value: f32) -> i32 {
    value.trunc().clamp(INT_MIN, INT_MAX) as i32
}

/// `round`, PLRM3's rule: "the integer value nearest to num1. If num1 is equally close to its
/// two nearest integers, round returns the greater of the two." Its own example is
/// `-6.5 round => -6.0`, which is neither `f32::round` nor WGSL's `round`.
#[must_use]
pub fn round(value: f32) -> f32 {
    let below = value.floor();
    if value - below >= 0.5 {
        below + 1.0
    } else {
        below
    }
}

/// `exp`, PLRM3's `base exponent exp`, whose own example `-9 -1 exp => -0.111111` proves a
/// negative base is defined for an integer exponent.
#[must_use]
pub fn exp(base: f32, exponent: f32) -> f32 {
    let magnitude = base.abs();
    if magnitude == 0.0 {
        return if exponent == 0.0 { 1.0 } else { 0.0 };
    }
    let size = (exponent * magnitude.log2()).exp2();
    if base > 0.0 {
        return size;
    }
    if exponent != exponent.trunc() {
        return 0.0;
    }
    if (exponent.abs() * 0.5).fract() == 0.0 {
        size
    } else {
        -size
    }
}

/// `bitshift`, PLRM3's "bits shifted out are lost; bits shifted in are 0" — so the right shift
/// is a logical one, not the arithmetic shift an `i32 >>` performs.
#[must_use]
pub fn bitshift(value: f32, shift: f32) -> f32 {
    let bits = to_int(value);
    let by = to_int(shift);
    if !(-31..=31).contains(&by) {
        return 0.0;
    }
    if by >= 0 {
        bits.wrapping_shl(by.unsigned_abs()) as f32
    } else {
        bits.cast_unsigned()
            .wrapping_shr(by.unsigned_abs())
            .cast_signed() as f32
    }
}

/// `atan`: PLRM3's angle in degrees in `[0, 360)`, with `0 0 atan` an `undefinedresult` that
/// the shader guards to zero.
#[must_use]
pub fn atan(num: f32, den: f32) -> f32 {
    if num == 0.0 && den == 0.0 {
        return 0.0;
    }
    let degrees = num.atan2(den).to_degrees();
    if degrees < 0.0 {
        degrees + 360.0
    } else {
        degrees
    }
}

/// Angle reduction to `[-180, 180)` degrees, matching `ps_reduce_degrees`.
#[must_use]
pub fn reduce_degrees(value: f32) -> f32 {
    value - 360.0 * ((value + 180.0) / 360.0).floor()
}

/// A value on the reference machine's operand stack.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Value {
    /// The number, which is what a colour is made of.
    pub number: f32,
    /// Which of §7.10.5.1's three types it is.
    pub kind: Kind,
}

/// ISO 32000-2 §7.10.5.1's three types: "expressions involving only integers, real numbers,
/// and boolean values".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A real number.
    Real,
    /// An integer.
    Integer,
    /// `true` or `false`.
    Boolean,
}

impl Value {
    pub fn real(number: f32) -> Self {
        Self {
            number,
            kind: Kind::Real,
        }
    }

    fn integer(number: f32) -> Self {
        Self {
            number,
            kind: Kind::Integer,
        }
    }

    fn boolean(state: bool) -> Self {
        Self {
            number: f32::from(state),
            kind: Kind::Boolean,
        }
    }

    /// The type an arithmetic operator leaves: an integer where both its operands were, a real
    /// otherwise. PLRM3 keeps `abs`, `neg`, `ceiling`, `floor`, `round`, `truncate`, `add`,
    /// `sub` and `mul` integer-preserving.
    fn numeric(number: f32, integral: bool) -> Self {
        if integral {
            Self::integer(number)
        } else {
            Self::real(number)
        }
    }
}

/// Evaluate a program at one point, returning the numbers it leaves on the operand stack.
///
/// No clipping: §7.10.1's output clip belongs to the shading, not to the program, and
/// [`evaluate_shading`] is where it is applied.
#[must_use]
pub fn evaluate(program: &[FnOp], x: f32, y: f32) -> Vec<f32> {
    evaluate_typed(program, x, y)
        .into_iter()
        .map(|value| value.number)
        .collect()
}

/// The same, with the types the machine tracked.
#[must_use]
pub fn evaluate_typed(program: &[FnOp], x: f32, y: f32) -> Vec<Value> {
    let mut stack = vec![Value::real(x), Value::real(y)];
    let mut pc = 0usize;
    // Forward-only jumping makes this bound unreachable; it costs one comparison per
    // instruction to be certain of that rather than to argue about it.
    let mut steps = 0usize;
    while let Some(op) = program.get(pc) {
        steps += 1;
        if steps > program.len() {
            break;
        }
        pc += 1;
        if let Some(target) = jump(*op, &mut stack) {
            pc = target;
            continue;
        }
        apply(*op, &mut stack);
    }
    stack
}

/// The whole shading at one point: §8.7.4.5.2's domain test, then the program, then §7.10.1's
/// output clip — as a colour and the coverage that goes with it.
///
/// **Outside the domain rectangle this discards rather than clamps.** ISO 32000-2 §8.7.4.5.2:
///
/// > Points within the shading's bounding box (BBox) that fall outside this transformed domain
/// > rectangle shall be painted with the shading's background colour (Background); if the
/// > shading dictionary has no Background entry, such points shall be left unpainted.
#[must_use]
pub fn evaluate_shading(witness: &Witness, x: f32, y: f32) -> [f32; 4] {
    let inside = x >= witness.domain.min.x
        && x <= witness.domain.max.x
        && y >= witness.domain.min.y
        && y <= witness.domain.max.y;
    if !inside {
        return match witness.background {
            Some(colour) => [colour.r, colour.g, colour.b, colour.a],
            None => [0.0; 4],
        };
    }
    let stack = evaluate(&witness.program, x, y);
    let [red, green, blue] = clip_outputs(&stack, witness.range);
    [red, green, blue, 1.0]
}

/// The top of an operand stack as a colour clipped to its `Range`.
#[must_use]
pub fn clip_outputs(stack: &[f32], range: FnRange) -> [f32; 3] {
    let (low, high) = range_bounds(range);
    let take = |back: usize| -> f32 {
        stack
            .get(stack.len().wrapping_sub(back))
            .copied()
            .unwrap_or(0.0)
    };
    match range {
        FnRange::Gray(_) => {
            let grey = take(1).clamp(low[0], high[0]);
            [grey, grey, grey]
        }
        FnRange::Rgb(_) => [
            take(3).clamp(low[0], high[0]),
            take(2).clamp(low[1], high[1]),
            take(1).clamp(low[2], high[2]),
        ],
    }
}

/// The two control-flow instructions, which are the only ones that move the counter.
fn jump(op: FnOp, stack: &mut Vec<Value>) -> Option<usize> {
    match op {
        FnOp::Jump { target } => Some(target as usize),
        FnOp::JumpUnless { target } => (pop(stack).number == 0.0).then_some(target as usize),
        _ => None,
    }
}

/// A pop of an empty operand stack yields **integer** `0`.
///
/// Both halves are decisions, not readings: ISO 32000-2 defines neither, and PostScript would
/// raise `stackunderflow`. The type matters as much as the value because seven of Table 42's
/// operators can tell an integer from a real.
pub fn pop(stack: &mut Vec<Value>) -> Value {
    stack.pop().unwrap_or(Value::integer(0.0))
}

fn unary(stack: &mut Vec<Value>, f: impl Fn(Value) -> Value) {
    let a = pop(stack);
    stack.push(f(a));
}

/// A one-operand operator that leaves a real whatever it consumed.
fn unary_real(stack: &mut Vec<Value>, f: impl Fn(f32) -> f32) {
    unary(stack, |a| Value::real(f(a.number)));
}

/// A one-operand operator PLRM3 keeps integer-preserving.
fn unary_numeric(stack: &mut Vec<Value>, f: impl Fn(f32) -> f32) {
    unary(stack, |a| {
        Value::numeric(f(a.number), a.kind == Kind::Integer)
    });
}

fn binary(stack: &mut Vec<Value>, f: impl Fn(Value, Value) -> Value) {
    let b = pop(stack);
    let a = pop(stack);
    stack.push(f(a, b));
}

fn binary_real(stack: &mut Vec<Value>, f: impl Fn(f32, f32) -> f32) {
    binary(stack, |a, b| Value::real(f(a.number, b.number)));
}

fn binary_numeric(stack: &mut Vec<Value>, f: impl Fn(f32, f32) -> f32) {
    binary(stack, |a, b| {
        Value::numeric(
            f(a.number, b.number),
            a.kind == Kind::Integer && b.kind == Kind::Integer,
        )
    });
}

fn binary_integer(stack: &mut Vec<Value>, f: impl Fn(i32, i32) -> i32) {
    binary(stack, |a, b| {
        Value::integer(f(to_int(a.number), to_int(b.number)) as f32)
    });
}

fn compare(stack: &mut Vec<Value>, f: impl Fn(f32, f32) -> bool) {
    binary(stack, |a, b| Value::boolean(f(a.number, b.number)));
}

/// `and`, `or` and `xor`: logical over two booleans, bitwise over two integers. With `true` as
/// 1 and `false` as 0 the two readings agree numerically, so only the *type* of the result
/// differs — which is why this is one function and not two.
fn logical_or_bitwise(stack: &mut Vec<Value>, f: impl Fn(i32, i32) -> i32) {
    binary(stack, |a, b| {
        let number = f(to_int(a.number), to_int(b.number)) as f32;
        if a.kind == Kind::Boolean && b.kind == Kind::Boolean {
            Value::boolean(number != 0.0)
        } else {
            Value::integer(number)
        }
    });
}

#[allow(
    clippy::too_many_lines,
    reason = "one arm per ISO 32000-2 Table 42 operator; the table is the function, and \
              splitting it across helpers would hide which operators are covered"
)]
pub fn apply(op: FnOp, stack: &mut Vec<Value>) {
    match op {
        FnOp::PushReal(value) => stack.push(Value::real(value)),
        FnOp::PushInt(value) => stack.push(Value::integer(value as f32)),
        FnOp::PushBool(value) => stack.push(Value::boolean(value)),

        FnOp::Abs => unary_numeric(stack, f32::abs),
        FnOp::Add => binary_numeric(stack, |a, b| a + b),
        FnOp::Atan => binary_real(stack, atan),
        FnOp::Ceiling => unary_numeric(stack, f32::ceil),
        FnOp::Cos => unary_real(stack, |a| reduce_degrees(a).to_radians().cos()),
        FnOp::Cvi => unary(stack, |a| Value::integer(a.number.trunc())),
        FnOp::Truncate => unary_numeric(stack, f32::trunc),
        FnOp::Cvr => unary(stack, |a| Value::real(a.number)),
        FnOp::Div => binary_real(stack, |a, b| if b == 0.0 { 0.0 } else { a / b }),
        FnOp::Exp => binary_real(stack, exp),
        FnOp::Floor => unary_numeric(stack, f32::floor),
        FnOp::Idiv => binary_integer(stack, |a, b| if b == 0 { 0 } else { a.wrapping_div(b) }),
        FnOp::Ln => unary_real(stack, |a| if a > 0.0 { a.ln() } else { 0.0 }),
        FnOp::Log => unary_real(stack, |a| {
            if a > 0.0 {
                a.log2() * core::f32::consts::LOG10_2
            } else {
                0.0
            }
        }),
        FnOp::Mod => binary_integer(stack, |a, b| if b == 0 { 0 } else { a.wrapping_rem(b) }),
        FnOp::Mul => binary_numeric(stack, |a, b| a * b),
        FnOp::Neg => unary_numeric(stack, |a| -a),
        FnOp::Round => unary_numeric(stack, round),
        FnOp::Sin => unary_real(stack, |a| reduce_degrees(a).to_radians().sin()),
        FnOp::Sqrt => unary_real(stack, |a| if a < 0.0 { 0.0 } else { a.sqrt() }),
        FnOp::Sub => binary_numeric(stack, |a, b| a - b),

        FnOp::And => logical_or_bitwise(stack, |a, b| a & b),
        FnOp::Or => logical_or_bitwise(stack, |a, b| a | b),
        FnOp::Xor => logical_or_bitwise(stack, |a, b| a ^ b),
        FnOp::Bitshift => binary(stack, |a, b| Value::integer(bitshift(a.number, b.number))),
        FnOp::Eq => compare(stack, |a, b| a == b),
        FnOp::Ne => compare(stack, |a, b| a != b),
        FnOp::Ge => compare(stack, |a, b| a >= b),
        FnOp::Gt => compare(stack, |a, b| a > b),
        FnOp::Le => compare(stack, |a, b| a <= b),
        FnOp::Lt => compare(stack, |a, b| a < b),
        // Table 42's `not`, resolved from the operand's run-time type rather than from a
        // static one. This is the arm the whole `Value` type exists for.
        FnOp::Not => unary(stack, |a| match a.kind {
            Kind::Boolean => Value::boolean(a.number == 0.0),
            _ => Value::integer(!to_int(a.number) as f32),
        }),

        FnOp::Copy => {
            let count = to_int(pop(stack).number);
            let depth = stack.len();
            if let Ok(count) = usize::try_from(count)
                && count <= depth
            {
                for index in depth.wrapping_sub(count)..depth {
                    stack.push(stack[index]);
                }
            }
        }
        FnOp::Dup => {
            let a = pop(stack);
            stack.push(a);
            stack.push(a);
        }
        FnOp::Exch => {
            let b = pop(stack);
            let a = pop(stack);
            stack.push(b);
            stack.push(a);
        }
        FnOp::Index => {
            let count = to_int(pop(stack).number);
            let from = usize::try_from(count)
                .ok()
                .and_then(|count| stack.len().checked_sub(count + 1));
            let value = from
                .and_then(|index| stack.get(index))
                .copied()
                .unwrap_or(Value::integer(0.0));
            stack.push(value);
        }
        FnOp::Pop => {
            pop(stack);
        }
        FnOp::Roll => {
            let by = to_int(pop(stack).number);
            let count = to_int(pop(stack).number);
            roll(stack, count, by);
        }

        FnOp::Jump { .. } | FnOp::JumpUnless { .. } => {}
    }
}

fn roll(stack: &mut [Value], count: i32, by: i32) {
    let Ok(count) = usize::try_from(count) else {
        return;
    };
    let depth = stack.len();
    if count == 0 || count > depth {
        return;
    }
    let Ok(count_i32) = i32::try_from(count) else {
        return;
    };
    let Ok(shift) = usize::try_from(by.rem_euclid(count_i32)) else {
        return;
    };
    stack[depth - count..depth].rotate_right(shift);
}

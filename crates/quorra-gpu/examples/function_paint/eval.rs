//! The same machine on the processor: the third implementation, and the oracle.
//!
//! It exists for two measurements, and it is written *independently* of the two
//! shaders rather than sharing a table with them — agreement between an independent
//! implementation and a device is evidence, agreement by construction is not
//! (the position `doc/adr/0037` takes in this tree for the same reason).
//!
//! 1. **What the arithmetic costs on the processor**, at the same placement, which is
//!    what the caller's 1 142 ms is a number for.
//! 2. **Whether the device agrees**, which is `QUORRA_FUNCTION_PAINT.md` §5.1 — the
//!    question they call the sharp one.

use crate::program::Op;

/// Evaluate a compiled program at one point, in f32.
///
/// `stack` is borrowed rather than allocated so the per-point cost is the arithmetic
/// and nothing else; the caller's evaluator allocates three times per device pixel,
/// and pricing their allocator is not what this spike is for.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per Table 42 operator; the table is the function"
)]
pub(crate) fn evaluate(ops: &[Op], x: f32, y: f32, stack: &mut Vec<f32>) -> [f32; 3] {
    stack.clear();
    stack.push(x);
    stack.push(y);

    let mut pc = 0usize;
    let mut steps = 0usize;
    while let Some(op) = ops.get(pc) {
        // Forward-only jumps make this unreachable; it costs one comparison to be
        // certain rather than to argue about it.
        steps = steps.saturating_add(1);
        if steps > ops.len() {
            break;
        }
        pc = pc.saturating_add(1);
        match *op {
            Op::PushReal(v) => stack.push(v),
            #[allow(
                clippy::cast_precision_loss,
                reason = "a literal beyond 2^24 is inexact in the compiled form itself"
            )]
            Op::PushInt(v) => stack.push(v as f32),
            Op::PushBool(v) => stack.push(f32::from(v)),

            Op::Abs => unary(stack, f32::abs),
            Op::Add => binary(stack, |a, b| a + b),
            Op::Atan => binary(stack, |a, b| {
                let degrees = a.atan2(b).to_degrees();
                if degrees < 0.0 {
                    degrees + 360.0
                } else {
                    degrees
                }
            }),
            Op::Ceiling => unary(stack, f32::ceil),
            Op::Cos => unary(stack, |a| a.to_radians().cos()),
            Op::Cvi | Op::Truncate => unary(stack, f32::trunc),
            Op::Cvr => {}
            Op::Div => binary(stack, |a, b| if b == 0.0 { 0.0 } else { a / b }),
            Op::Exp => binary(stack, f32::powf),
            Op::Floor => unary(stack, f32::floor),
            Op::IDiv => binary(stack, |a, b| {
                let d = b.trunc();
                if d == 0.0 {
                    0.0
                } else {
                    (a.trunc() / d).trunc()
                }
            }),
            Op::Ln => unary(stack, |a| if a > 0.0 { a.ln() } else { 0.0 }),
            Op::Log => unary(stack, |a| if a > 0.0 { a.log10() } else { 0.0 }),
            Op::Mod => binary(stack, |a, b| {
                let d = b.trunc();
                if d == 0.0 { 0.0 } else { a.trunc() % d }
            }),
            Op::Mul => binary(stack, |a, b| a * b),
            Op::Neg => unary(stack, |a| -a),
            // Half away from zero, as PostScript's `round` is.
            Op::Round => unary(stack, |a| a.signum() * (a.abs() + 0.5).floor()),
            Op::Sin => unary(stack, |a| a.to_radians().sin()),
            Op::Sqrt => unary(stack, |a| if a > 0.0 { a.sqrt() } else { 0.0 }),
            Op::Sub => binary(stack, |a, b| a - b),

            Op::And => bitwise(stack, |a, b| a & b),
            Op::Or => bitwise(stack, |a, b| a | b),
            Op::Xor => bitwise(stack, |a, b| a ^ b),
            Op::BitShift => binary(stack, |a, b| {
                let (value, shift) = (to_int(a), to_int(b));
                let result = if shift >= 32 || shift <= -32 {
                    0
                } else if shift >= 0 {
                    value.wrapping_shl(shift.unsigned_abs())
                } else {
                    value.wrapping_shr(shift.unsigned_abs())
                };
                from_int(result)
            }),
            // Exact equality, which is what Table 42 says `eq` is. The caller's
            // evaluator compares within an absolute `f32::EPSILON` instead; that
            // difference is reported in the write-up, not copied here (principle 5).
            #[allow(clippy::float_cmp, reason = "Table 42's `eq` is exact equality")]
            Op::Eq => binary(stack, |a, b| f32::from(a == b)),
            #[allow(clippy::float_cmp, reason = "Table 42's `ne` is exact inequality")]
            Op::Ne => binary(stack, |a, b| f32::from(a != b)),
            Op::Ge => binary(stack, |a, b| f32::from(a >= b)),
            Op::Gt => binary(stack, |a, b| f32::from(a > b)),
            Op::Le => binary(stack, |a, b| f32::from(a <= b)),
            Op::Lt => binary(stack, |a, b| f32::from(a < b)),
            Op::Not => unary(stack, |a| f32::from(a == 0.0)),
            Op::BitNot => unary(stack, |a| from_int(!to_int(a))),

            Op::Copy => {
                let n = to_int(pop(stack));
                let depth = stack.len();
                if let Ok(n) = usize::try_from(n)
                    && n <= depth
                {
                    for k in depth.saturating_sub(n)..depth {
                        stack.push(stack[k]);
                    }
                }
            }
            Op::Dup => {
                let a = pop(stack);
                stack.push(a);
                stack.push(a);
            }
            Op::Exch => {
                let b = pop(stack);
                let a = pop(stack);
                stack.push(b);
                stack.push(a);
            }
            Op::Index => {
                let n = to_int(pop(stack));
                let from = usize::try_from(n)
                    .ok()
                    .and_then(|n| stack.len().checked_sub(n.saturating_add(1)));
                stack.push(from.map_or(0.0, |index| stack[index]));
            }
            Op::Pop => {
                pop(stack);
            }
            Op::Roll => {
                let by = to_int(pop(stack));
                let count = to_int(pop(stack));
                roll(stack, count, by);
            }

            Op::JumpIfFalse(target) => {
                if pop(stack) == 0.0 {
                    pc = target as usize;
                }
            }
            Op::Jump(target) => pc = target as usize,
        }
    }

    let blue = pop(stack);
    let green = pop(stack);
    let red = pop(stack);
    [red, green, blue]
}

/// A pop of an empty stack yields 0 — see the note in `interp.wgsl`.
fn pop(stack: &mut Vec<f32>) -> f32 {
    stack.pop().unwrap_or(0.0)
}

fn unary(stack: &mut Vec<f32>, f: impl Fn(f32) -> f32) {
    let a = pop(stack);
    stack.push(f(a));
}

fn binary(stack: &mut Vec<f32>, f: impl Fn(f32, f32) -> f32) {
    let b = pop(stack);
    let a = pop(stack);
    stack.push(f(a, b));
}

fn bitwise(stack: &mut Vec<f32>, f: impl Fn(i32, i32) -> i32) {
    binary(stack, |a, b| from_int(f(to_int(a), to_int(b))));
}

/// Saturating, for the reason `ops.wgsl` gives: a wrapped integer is a plausible
/// wrong colour, and §6 of the brief prices that above a refusal.
#[allow(
    clippy::cast_possible_truncation,
    reason = "the clamp is what makes the truncation exact"
)]
fn to_int(a: f32) -> i32 {
    a.trunc().clamp(-2_147_483_648.0, 2_147_483_647.0) as i32
}

#[allow(
    clippy::cast_precision_loss,
    reason = "the same loss the device's f32 stack has, deliberately"
)]
fn from_int(a: i32) -> f32 {
    a as f32
}

fn roll(stack: &mut [f32], count: i32, by: i32) {
    let Ok(n) = usize::try_from(count) else {
        return;
    };
    let depth = stack.len();
    if n == 0 || n > depth {
        return;
    }
    let Ok(shift) = usize::try_from(by.rem_euclid(count)) else {
        return;
    };
    stack[depth.saturating_sub(n)..depth].rotate_right(shift);
}

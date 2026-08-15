//! Table 42'''s typing rules: which operator a token is, what type it leaves, and where a
//! wrong type is a refusal rather than an inference.
//!
//! One responsibility: **§7.10.5.1'''s three types, as static rules.** The clause gives the
//! calculator "expressions involving only integers, real numbers, and boolean values", and
//! PLRM3 makes several of Table 42'''s operators mean different things over two of them. The
//! operand stack carries one representation, so a type is a static fact here or it is not a
//! fact at all.
//!
//! # The line this file draws, stated once
//!
//! **A type is enforced where lowering a wrongly-typed operand would silently compute a
//! different function, and inferred but not enforced everywhere else.**
//!
//! `idiv`, `mod`, `bitshift`, `and`, `or` and `xor` truncate their operands to integers, so a
//! real reaching one loses its fractional part and paints a plausible colour that no reading
//! of PLRM3 produces; `not` is two operators wearing one name and a wrong guess is a wrong
//! colour; `if` branches on a value the standard only ever calls a boolean. Those are
//! enforced. `abs` of a boolean is a `typecheck` in PostScript too — and there is nothing for
//! us to get wrong about it, so it is not our business to police.

use quorra_scene::FnOp;

use super::lowered::SlotType;
use super::operators::{Binary, Unary};
use super::refusal::FunctionRefusal;

/// The one-operand Table 42 operators that need no type resolution.
pub(super) fn unary_of(op: FnOp) -> Option<Unary> {
    Some(match op {
        FnOp::Abs => Unary::Abs,
        FnOp::Ceiling => Unary::Ceiling,
        FnOp::Cos => Unary::Cos,
        FnOp::Cvi => Unary::Cvi,
        FnOp::Cvr => Unary::Cvr,
        FnOp::Floor => Unary::Floor,
        FnOp::Ln => Unary::Ln,
        FnOp::Log => Unary::Log,
        FnOp::Neg => Unary::Neg,
        FnOp::Round => Unary::Round,
        FnOp::Sin => Unary::Sin,
        FnOp::Sqrt => Unary::Sqrt,
        FnOp::Truncate => Unary::Truncate,
        _ => return None,
    })
}

/// The two-operand Table 42 operators.
pub(super) fn binary_of(op: FnOp) -> Option<Binary> {
    Some(match op {
        FnOp::Add => Binary::Add,
        FnOp::Atan => Binary::Atan,
        FnOp::Div => Binary::Div,
        FnOp::Exp => Binary::Exp,
        FnOp::Idiv => Binary::Idiv,
        FnOp::Mod => Binary::Mod,
        FnOp::Mul => Binary::Mul,
        FnOp::Sub => Binary::Sub,
        FnOp::And => Binary::And,
        FnOp::Bitshift => Binary::Bitshift,
        FnOp::Eq => Binary::Eq,
        FnOp::Ge => Binary::Ge,
        FnOp::Gt => Binary::Gt,
        FnOp::Le => Binary::Le,
        FnOp::Lt => Binary::Lt,
        FnOp::Ne => Binary::Ne,
        FnOp::Or => Binary::Or,
        FnOp::Xor => Binary::Xor,
        _ => return None,
    })
}

/// The type a one-operand operator leaves, given the type it consumed.
pub(super) fn unary_result(op: Unary, operand: SlotType) -> SlotType {
    match op {
        // PLRM3 keeps these integer-preserving: `abs`, `neg`, `ceiling`, `floor`, `round`
        // and `truncate` return an integer for an integer operand and a real for a real.
        Unary::Abs
        | Unary::Neg
        | Unary::Ceiling
        | Unary::Floor
        | Unary::Round
        | Unary::Truncate => match operand {
            SlotType::Integer => SlotType::Integer,
            // A number out of a boolean is a number, not a boolean.
            SlotType::Real | SlotType::Boolean => SlotType::Real,
            SlotType::Undecided => SlotType::Undecided,
        },
        Unary::Cvi | Unary::BitwiseNot => SlotType::Integer,
        Unary::LogicalNot => SlotType::Boolean,
        Unary::Cvr | Unary::Cos | Unary::Ln | Unary::Log | Unary::Sin | Unary::Sqrt => {
            SlotType::Real
        }
    }
}

/// The type a two-operand operator leaves.
pub(super) fn binary_result(op: Binary, left: SlotType, right: SlotType) -> SlotType {
    match op {
        // §7.10.5.2 and PLRM3: integer-preserving when both operands are integers.
        Binary::Add | Binary::Mul | Binary::Sub => {
            if left == SlotType::Undecided || right == SlotType::Undecided {
                SlotType::Undecided
            } else if left == SlotType::Integer && right == SlotType::Integer {
                SlotType::Integer
            } else {
                SlotType::Real
            }
        }
        // "div: divides num1 by num2, producing a result that is always a real number even
        // if both operands are integers."
        Binary::Div | Binary::Atan | Binary::Exp => SlotType::Real,
        Binary::Idiv | Binary::Mod | Binary::Bitshift => SlotType::Integer,
        Binary::And | Binary::Or | Binary::Xor => {
            if left == SlotType::Boolean {
                SlotType::Boolean
            } else {
                SlotType::Integer
            }
        }
        Binary::Eq | Binary::Ge | Binary::Gt | Binary::Le | Binary::Lt | Binary::Ne => {
            SlotType::Boolean
        }
    }
}

/// The type requirement of the operators whose lowering would otherwise compute something
/// ISO 32000-2 does not define.
///
/// The line this draws, stated once: **a type is enforced where lowering a wrongly-typed
/// operand would silently compute a different function**, and inferred but not enforced
/// everywhere else. `idiv`, `mod`, `bitshift`, `and`, `or` and `xor` truncate their operands
/// to integers, so a real reaching one loses its fractional part and paints a plausible
/// colour that no reading of PLRM3 produces. `abs` of a boolean is a `typecheck` in
/// PostScript too, but there is nothing for us to get wrong about it, so it is not our
/// business to police.
pub(super) fn check_operand_types(
    op: Binary,
    left: SlotType,
    right: SlotType,
) -> Result<(), FunctionRefusal> {
    let (required, admitted): (&'static str, fn(SlotType) -> bool) = match op {
        // PLRM3: "Both operands of idiv must be integers"; the same for `mod` and for
        // `bitshift`'s value and shift.
        Binary::Idiv | Binary::Mod | Binary::Bitshift => {
            ("two integers", |ty| ty == SlotType::Integer)
        }
        // Table 42: logical on two booleans, bitwise on two integers. With `true` as 1 and
        // `false` as 0 the two readings agree, which is why one lowering serves both — but
        // neither reading covers a real.
        Binary::And | Binary::Or | Binary::Xor => ("two booleans or two integers", |ty| {
            matches!(ty, SlotType::Boolean | SlotType::Integer)
        }),
        _ => return Ok(()),
    };
    if left == SlotType::Boolean || right == SlotType::Boolean {
        // A mixed boolean/integer pair is a `typecheck` in PostScript and has no single
        // reading here either; `admitted` alone would let it through for and/or/xor.
        if left != right {
            return Err(mismatch(op, required, left, right));
        }
    }
    for ty in [left, right] {
        if ty == SlotType::Undecided {
            return Err(FunctionRefusal::UndecidableOperandType {
                operator: op.table_42(),
            });
        }
        if !admitted(ty) {
            return Err(FunctionRefusal::OperandType {
                operator: op.table_42(),
                required,
                found: ty.name(),
            });
        }
    }
    Ok(())
}

fn mismatch(
    op: Binary,
    required: &'static str,
    left: SlotType,
    right: SlotType,
) -> FunctionRefusal {
    let found = if left == SlotType::Boolean {
        right
    } else {
        left
    };
    if found == SlotType::Undecided {
        return FunctionRefusal::UndecidableOperandType {
            operator: op.table_42(),
        };
    }
    FunctionRefusal::OperandType {
        operator: op.table_42(),
        required,
        found: found.name(),
    }
}

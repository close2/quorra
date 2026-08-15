//! Why a program is declined, by name.
//!
//! `QUORRA_FUNCTION_PAINT.md` §5.2: "a refusal by name — `UnsupportedPaint` or its
//! successor — for any program the device declines, so this tree can fall back to the
//! raster it builds today rather than draw nothing." This module is that list, and
//! `main`'s refusal table demonstrates every ground on a program that reaches it: a
//! ground nobody can reach is not a ground.
//!
//! Two of the grounds belong to the generated shader alone
//! ([`Refusal::UnbalancedBranches`], [`Refusal::DynamicStackOperand`]) and are named
//! as such, because which shape refuses what is half the spike's answer.

use std::fmt;

/// Why a program cannot be evaluated, by name.
///
/// `QUORRA_FUNCTION_PAINT.md` §5.2 asks for "a refusal by name … for any program the
/// device declines"; these are the grounds this spike found it needs. Each names what
/// exceeded what, per CLAUDE.md principle 6.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A token that is not a Table 42 operator and not a number.
    UnknownOperator(String),
    /// `{` without a matching `}`, or the reverse.
    UnbalancedBraces,
    /// A procedure literal that is not the operand of `if` or `ifelse`. §7.10.5.1
    /// admits procedures nowhere else.
    StrayProcedure,
    /// `if` or `ifelse` without the procedures it needs immediately before it.
    MalformedConditional,
    /// The operand stack would exceed the depth the shader has slots for.
    StackTooDeep {
        /// What the program needs.
        needed: usize,
        /// What the shader provides.
        limit: usize,
    },
    /// The two branches of an `ifelse` leave different stack depths, so no static
    /// slot assignment describes the join. Only the generated shader minds.
    UnbalancedBranches {
        /// Depth after the `if` branch.
        then_depth: usize,
        /// Depth after the `else` branch.
        else_depth: usize,
    },
    /// `copy`, `index` or `roll` whose count is not a literal, so a generated shader
    /// cannot name the slots it touches. Only the generated shader minds.
    DynamicStackOperand(&'static str),
    /// A type-sensitive operator (`and`, `or`, `xor`, `not`) reached a value whose
    /// kind two branches disagreed about.
    AmbiguousKind(&'static str),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOperator(t) => write!(f, "`{t}` is not an ISO 32000-2 Table 42 operator"),
            Self::UnbalancedBraces => write!(f, "unbalanced `{{` and `}}`"),
            Self::StrayProcedure => write!(f, "a procedure that is not an `if`/`ifelse` operand"),
            Self::MalformedConditional => write!(f, "`if`/`ifelse` without its procedures"),
            Self::StackTooDeep { needed, limit } => {
                write!(
                    f,
                    "operand stack needs {needed} slots, the shader has {limit}"
                )
            }
            Self::UnbalancedBranches {
                then_depth,
                else_depth,
            } => write!(
                f,
                "an `ifelse` leaves {then_depth} operands on one branch and {else_depth} on the other"
            ),
            Self::DynamicStackOperand(op) => {
                write!(f, "`{op}` with a count that is not a literal")
            }
            Self::AmbiguousKind(op) => write!(f, "`{op}` on a value of two different types"),
        }
    }
}

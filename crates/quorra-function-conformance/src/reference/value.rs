//! The three types a §7.10.5 program computes with, and the coercions between them.
//!
//! ISO 32000-2 §7.10.5.1 names them and no others:
//!
//! > Expressions involving only integers, real numbers, and boolean values
//!
//! Keeping them apart is not pedantry. Half of Table 42's sharp cases are type
//! questions wearing arithmetic clothes — `not` is one operator on a boolean and a
//! different one on an integer, `idiv` and `mod` refuse a real, `ceiling` returns
//! whichever type it was given, and `div` returns a real even when both operands were
//! integers. An evaluator over `f32` alone answers several of them wrongly and cannot
//! notice.

use crate::case::PsError;

/// A value on the operand stack.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    /// An integer. PLRM3 Appendix B's `integer` is 32-bit two's complement, and
    /// ISO 32000-2 Table C.1 says the same informatively: "Integer values (such as
    /// object numbers) can often be expressed within 32 bits."
    Int(i32),
    /// A real. ISO 32000-2 §7.3.3 defers its representation to the machine; this
    /// evaluator uses `f32`, which is the width the device computes in, and the choice
    /// is recorded in `doc/notes-function-conformance.md` rather than implied here.
    Real(f32),
    /// A boolean, which §7.10.5.3 forbids as an *output* — "It shall be an error … for
    /// any of them to be objects other than numbers" — but permits everywhere inside.
    Bool(bool),
}

impl Value {
    /// The value's mathematical value, for the operators PLRM3 defines over `num`.
    ///
    /// `f64` because it holds every `i32` and every `f32` exactly, so a comparison
    /// between an integer and a real is the comparison of their mathematical values —
    /// which is what PLRM3's `eq` entry asks for — rather than of one lossy conversion.
    ///
    /// # Errors
    ///
    /// `typecheck` for a boolean: PLRM3's `gt`, `ge`, `lt` and `le` entries say "If the
    /// operands are of other types … a typecheck error occurs".
    pub fn number(self) -> Result<f64, PsError> {
        match self {
            Self::Int(i) => Ok(f64::from(i)),
            Self::Real(r) => Ok(f64::from(r)),
            Self::Bool(_) => Err(PsError::TypeCheck),
        }
    }

    /// The value as an integer, for the operators PLRM3 defines over `int`.
    ///
    /// # Errors
    ///
    /// `typecheck` for a real or a boolean. PLRM3's `idiv` entry: "Both operands of
    /// idiv must be integers"; its `bitshift` entry: "Both int1 and shift must be
    /// integers".
    pub fn integer(self) -> Result<i32, PsError> {
        match self {
            Self::Int(i) => Ok(i),
            Self::Real(_) | Self::Bool(_) => Err(PsError::TypeCheck),
        }
    }

    /// The value as a boolean, for `if` and `ifelse`'s condition.
    ///
    /// # Errors
    ///
    /// `typecheck` for a number. PLRM3's `if` entry lists `typecheck` among its errors,
    /// and its operand is `bool`.
    pub fn boolean(self) -> Result<bool, PsError> {
        match self {
            Self::Bool(b) => Ok(b),
            Self::Int(_) | Self::Real(_) => Err(PsError::TypeCheck),
        }
    }

    /// The value as an `f32`, for an operator whose result PLRM3 says "is always a real
    /// number".
    ///
    /// # Errors
    ///
    /// `typecheck` for a boolean.
    pub fn real(self) -> Result<f32, PsError> {
        match self {
            // PLRM3's `cvr` entry: "If the operand is an integer, cvr converts it to a
            // real number." An integer above 2^24 does not survive the conversion
            // exactly, and that is the conversion the clause asks for.
            #[allow(clippy::cast_precision_loss)]
            Self::Int(i) => Ok(i as f32),
            Self::Real(r) => Ok(r),
            Self::Bool(_) => Err(PsError::TypeCheck),
        }
    }

    /// Whether this is a number, which is what §7.10.5.3 requires of every value a
    /// program leaves behind.
    #[must_use]
    pub const fn is_number(self) -> bool {
        matches!(self, Self::Int(_) | Self::Real(_))
    }
}

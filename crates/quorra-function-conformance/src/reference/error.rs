//! What the reference evaluator returns instead of a number, and the three different
//! things that can mean.
//!
//! Keeping them apart is the whole discipline of this crate. "PLRM3 says this is an
//! error", "neither document says anything" and "this is not a well-formed compiled
//! program" are three different findings, and collapsing them into one would let the
//! second one — the only one that is a fact about the *standard* rather than about an
//! input — disappear into a catch-all.

use crate::case::PsError;

/// Why an evaluation produced no outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EvalError {
    /// The operator's PLRM3 entry names this error for these operands, and no document
    /// defines a value.
    #[error("{0:?}: PLRM3 names this error for these operands and defines no result")]
    Error(PsError),
    /// Neither ISO 32000-2 nor PLRM3 defines a result here. The evaluator declines to
    /// invent one; the string says what was read and what it did not say.
    #[error("the specification defines nothing here: {0}")]
    Undefined(&'static str),
    /// The instruction list is not a well-formed compiled §7.10.5 function — a backward
    /// jump, a target past the end, more inputs than the domain has pairs. This is a
    /// defect in whoever built the program, not a property of the standard.
    #[error("not a well-formed compiled §7.10.5 program: {0}")]
    Malformed(&'static str),
}

impl From<PsError> for EvalError {
    fn from(error: PsError) -> Self {
        Self::Error(error)
    }
}

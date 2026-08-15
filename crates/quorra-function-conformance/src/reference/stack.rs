//! The operand stack, its normative depth, and the one place a choice is made where
//! the standard says nothing.
//!
//! ISO 32000-2 §7.10.5.3, verbatim:
//!
//! > Implementations of Type 4 functions shall provide a stack with room for at least
//! > 100 entries. No implementation shall be required to provide a larger stack, and it
//! > shall be an error to overflow the stack.
//!
//! So 100 is a floor on what an implementation must provide and a ceiling on what a
//! program may assume. This stack holds exactly 100, which makes it the strictest
//! conforming implementation and therefore the right oracle: a program this refuses is
//! a program some conforming processor refuses.

use super::error::EvalError;
use super::value::Value;
use crate::case::{PsError, Report};

/// §7.10.5.3's floor, used here as the exact capacity.
pub const CAPACITY: usize = 100;

/// The operand stack of one evaluation, with the reports it accumulated.
#[derive(Debug, Default)]
pub struct Stack {
    values: Vec<Value>,
    reports: Vec<Report>,
}

impl Stack {
    /// An empty stack.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a value.
    ///
    /// # Errors
    ///
    /// [`PsError::StackOverflow`] past [`CAPACITY`], which §7.10.5.3 makes an error
    /// rather than a growth.
    pub fn push(&mut self, value: Value) -> Result<(), EvalError> {
        if self.values.len() >= CAPACITY {
            return Err(PsError::StackOverflow.into());
        }
        self.values.push(value);
        Ok(())
    }

    /// Pop a value, or **0 from an empty stack**.
    ///
    /// This is the one place the evaluator answers a question no document asks. Popping
    /// an empty stack raises `stackunderflow` in PostScript and ISO 32000-2 defines
    /// nothing; the pinned vocabulary's decision 6 takes the caller's reading, because
    /// their `pi_seven_segment.pdf` depends on it three times and refusing it would mean
    /// refusing a witness. A [`Report::EmptyStackPop`] is recorded so that the choice
    /// travels with the frame instead of being adopted invisibly.
    ///
    /// **The 0 is an integer**, and that is a second choice inside the first: a `0`
    /// written in a PostScript program scans as an integer, and an integer coerces into
    /// every numeric context where a real does not — `and`, `not`, `idiv`, `mod` and
    /// `bitshift` all reject a real. `doc/notes-function-conformance.md` records it as a
    /// question for the caller rather than as a reading of any clause.
    pub fn pop(&mut self) -> Value {
        let Some(value) = self.values.pop() else {
            self.report(Report::EmptyStackPop);
            return Value::Int(0);
        };
        value
    }

    /// How many values are on the stack.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.values.len()
    }

    /// The value `n` places below the top, where 0 is the top itself.
    #[must_use]
    pub fn peek(&self, n: usize) -> Option<Value> {
        let index = self.values.len().checked_sub(1)?.checked_sub(n)?;
        self.values.get(index).copied()
    }

    /// Circularly shift the top `n` values by `j`, positive `j` moving them toward the
    /// top, which is PLRM3's `roll`.
    ///
    /// # Errors
    ///
    /// [`PsError::StackUnderflow`], which PLRM3's `roll` entry lists, when fewer than
    /// `n` values are present. The zero-from-an-empty-stack rule is deliberately *not*
    /// extended here: it is a rule about popping, and inventing operands for a rotation
    /// would be a second invention on top of the first.
    pub fn rotate_top(&mut self, n: usize, j: i32) -> Result<(), EvalError> {
        if n > self.values.len() {
            return Err(PsError::StackUnderflow.into());
        }
        if n == 0 {
            return Ok(());
        }
        let Ok(width) = i32::try_from(n) else {
            return Err(PsError::RangeCheck.into());
        };
        // `j` is any integer and `n` is positive, so the two remainders below cannot
        // divide by zero, and `rem_euclid` on a positive divisor is in `0..n`.
        let shift = usize::try_from(j.rem_euclid(width)).unwrap_or(0);
        let base = self.values.len().saturating_sub(n);
        self.values[base..].rotate_right(shift);
        Ok(())
    }

    /// Record a choice the frame must carry, once per kind.
    pub fn report(&mut self, report: Report) {
        if !self.reports.contains(&report) {
            self.reports.push(report);
        }
    }

    /// The values left behind, which §7.10.5.3 makes the output variables.
    #[must_use]
    pub fn values(&self) -> &[Value] {
        &self.values
    }

    /// The choices this evaluation relied on.
    #[must_use]
    pub fn reports(&self) -> &[Report] {
        &self.reports
    }
}

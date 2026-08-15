//! The ISO 32000-2 §7.10.5 conformance corpus, and the reference evaluator that is its
//! oracle.
//!
//! `doc/adr/0053` decided that a §8.7.4.5.2 type 1 shading whose function is a type 4
//! program is worth evaluating on the device, and closed with the condition this crate
//! satisfies: *"the classification needs a conformance test per dangerous and per safe
//! operator before it is a contract"*. A corpus is what turns a reading of a clause into
//! something a later wave can fail against.
//!
//! # What is here
//!
//! - [`table42`] — the 42 operator names, so that "every operator has a case" is a test.
//! - [`case`] — what a case is. Every one carries a `citation`: the clause or PLRM3
//!   entry its expected value came from, and, where the reading is ours rather than the
//!   document's, that it is ours. CLAUDE.md principle 5 asks for that as a comment; a
//!   field is a comment nobody can forget.
//! - [`corpus`] — the cases themselves, one family per module: at least one program per
//!   Table 42 operator, the cases where implementations drift, §7.10's two clips, and one
//!   program per refusal ground.
//! - [`reference`] — an evaluator written from the clause, which the corpus's own test
//!   runs every case through. Where an expectation and the evaluator disagree, one of
//!   them has misread ISO 32000-2 or PLRM3, and finding out which is the point.
//!
//! # The four things a case can expect, and why there are four
//!
//! A corpus that only holds values is a corpus that has quietly decided every question
//! the standard left open. [`case::Expectation`] has four states so that the interesting
//! ones survive being written down:
//!
//! | | |
//! |---|---|
//! | [`Outputs`](case::Expectation::Outputs) | the clause fixes the values |
//! | [`Error`](case::Expectation::Error) | PLRM3 names an error and defines no value |
//! | [`Undefined`](case::Expectation::Undefined) | **neither document defines anything**, and the case says so instead of guessing |
//! | [`Refused`](case::Expectation::Refused) | the program is refused before a frame is drawn |
//!
//! The third is the one this crate exists to protect. `doc/notes-function-conformance.md`
//! lists every case that lands there.
//!
//! # Running a case on a device
//!
//! A `Paint::Function` is a two-input function; most operator cases take no inputs and
//! push their own literals, which is a legal §7.10.5 function of zero inputs but not a
//! legal shading. [`case::Case::two_input_program`] is the adaptor, and it is here
//! rather than in the wave that needs it because prepending instructions moves every
//! jump target.

#![forbid(unsafe_code)]

pub mod case;
pub mod corpus;
pub mod reference;
pub mod table42;

pub use case::{Case, Expectation, PsError, Refusal, Report, Subject, Tolerance};
pub use corpus::{Family, cases};
pub use reference::{Evaluation, evaluate, evaluate_case};
pub use table42::Table42;

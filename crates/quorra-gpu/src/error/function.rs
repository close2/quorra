//! Why a program the device would have evaluated is not admitted.
//!
//! The vocabulary of
//! [`DeviceError::InvalidFunction`](crate::error::DeviceError::InvalidFunction), which is
//! ISO 32000-2 §7.10.5's type 4 (PostScript calculator) function asked the three
//! questions of `function::admit` and answered no to one of them (ADR 0053).
//!
//! Its own module rather than a neighbour of
//! [`ResourceProblem`](crate::error::ResourceProblem), for the same reason it is its own
//! variant of `DeviceError`: a ramp is refused for what its numbers *are*, a program for
//! what it would *do*, and the three answers here are each in a different subsystem's
//! vocabulary.

use thiserror::Error;

/// Why a device will not execute a §7.10.5 type 4 program (ADR 0053).
///
/// Three variants for the three questions `function::admit` asks, in its order: is the
/// instruction list executable, can a shader be generated from it, and can an independent
/// evaluation of it be expected to agree with ours. Each carries the refusal that answered
/// no, in that refusal's own vocabulary, because translating one into the other would put a
/// second definition of "a program this device declines" into the tree.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum FunctionProblem {
    /// The instruction list is not structurally executable: empty, past
    /// [`quorra_scene::MAX_PROGRAM_LENGTH`], or carrying a jump that does not move
    /// strictly forward into it.
    #[error("{0}")]
    Structure(quorra_scene::SceneError),
    /// The walk that admits a program refused it — see [`FunctionRefusal`] for the ground.
    ///
    /// [`FunctionRefusal`]: crate::function::FunctionRefusal
    #[error("{0}")]
    Program(crate::function::FunctionRefusal),
    /// An operator whose two implementations may differ reaches one that turns a small
    /// difference into a large one, so no bound on the disagreement between this device
    /// and an independent evaluation can be stated (ADR 0053 §3).
    ///
    /// Refused at the upload rather than reported on the frame: the caller's answer is to
    /// fall back to the raster it builds today, and that is cheap here and expensive after
    /// a page has been drawn.
    #[error(
        "`{inexact}` at {inexact_at} reaches `{amplifier}` at {amplifier_at}, so no bound \
         on the disagreement with an independent evaluation can be stated"
    )]
    NoAgreementBound {
        /// The inexact operator, as ISO 32000-2 Table 42 spells it.
        inexact: &'static str,
        /// Where it is, as an index into the program.
        inexact_at: usize,
        /// The operator whose result its value reaches.
        amplifier: &'static str,
        /// Where that one is.
        amplifier_at: usize,
    },
}

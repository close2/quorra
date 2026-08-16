//! What an upload's content violated, in the words the upload itself uses.
//!
//! The vocabulary of
//! [`DeviceError::InvalidResource`](crate::error::DeviceError::InvalidResource): one
//! variant per question the validators in `resources.rs` ask of an outline, an image
//! or a ramp *before* anything is stored — the whole enum is raised in that one file
//! and nowhere else. §4.7 of the brief is why they are questions at all — a coordinate of
//! 1e30 and a 60 000×60 000 image both arrive from real files by way of a correct
//! interpreter, and the answer is a refusal by name rather than a repair.

use thiserror::Error;

/// What exactly an upload violated. Enumerated for the same reason `ReportKind` is:
/// "how often does this happen?" must stay answerable.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum ResourceProblem {
    /// An outline with no segments at all.
    #[error("outline has no segments")]
    OutlineEmpty,
    /// An outline whose first segment is not a `MoveTo` — no current point exists for
    /// anything else to draw from (ISO 32000-2 §8.5.2's path construction starts
    /// every path with `m`).
    #[error("outline does not start with MoveTo")]
    OutlineMissingMoveTo,
    /// An outline containing a NaN or infinite coordinate.
    #[error("outline has a non-finite coordinate")]
    OutlineNonFinite,
    /// An outline coordinate beyond the scene coordinate limit.
    #[error("outline coordinate exceeds the limit of {limit}")]
    OutlineCoordinateTooLarge {
        /// The limit (`quorra_scene::MAX_COORDINATE`).
        limit: f32,
    },
    /// An image whose dimensions and byte length disagree, or with a zero dimension.
    #[error("image is {width}x{height} but carries {bytes} bytes")]
    ImageInconsistent {
        /// Claimed width.
        width: u32,
        /// Claimed height.
        height: u32,
        /// Actual byte length.
        bytes: usize,
    },
    /// A ramp with no stops.
    #[error("ramp has no stops")]
    RampEmpty,
    /// A ramp stop offset that is NaN, infinite, or outside `0..=1`.
    #[error("ramp stop offset {offset} is outside 0..=1")]
    RampOffsetOutOfRange {
        /// The offending offset.
        offset: f32,
    },
    /// Ramp stops out of ascending order.
    #[error("ramp stops are not in ascending offset order")]
    RampUnordered,
    /// A ramp stop colour whose red, green, blue or alpha component is NaN, infinite, or
    /// outside `0..=1` — the four questions
    /// [`Color::is_valid`](quorra_scene::Color::is_valid) asks, all of them.
    #[error("ramp stop colour is non-finite or outside 0..=1")]
    RampColorInvalid,
}

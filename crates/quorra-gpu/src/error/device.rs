//! What a device refuses when no frame is in flight.
//!
//! One enum, and every call that can return it is outside a frame: the four
//! constructors (§2.1 of the brief), the five `upload_*` methods and `release`
//! (§2.2). A frame's refusals are [`RenderError`](crate::error::RenderError) — a
//! different phase, with different fixes, and the reason the two are not one enum.
//!
//! Two variants delegate the *why* to a vocabulary of their own — an upload's content
//! to [`ResourceProblem`], a program to [`FunctionProblem`] — so that what a caller
//! matches on stays a list of situations while the reasons stay countable.

use thiserror::Error;

use super::function::FunctionProblem;
use super::resource::ResourceProblem;

/// Why a device could not be constructed, or would not take or give up a resource.
///
/// Every variant names what was unavailable, what the budget was and what asked to
/// exceed it, or which identifier was not resident. The first four are construction and
/// the last five are residency; both are outside any frame, which is what makes them one
/// enum rather than two.
#[derive(Debug, Error)]
pub enum DeviceError {
    /// No adapter matched the request. Carries every adapter that was available so
    /// the caller (or the person reading the error) can see what would have matched.
    #[error("no adapter matched {requested:?}; adapters present: {available:?}")]
    NoAdapter {
        /// The `Options::adapter` filter that was applied, if any.
        requested: Option<String>,
        /// The names of every adapter enumeration found.
        available: Vec<String>,
    },
    /// The adapter refused to yield a device.
    #[error("device creation failed on adapter '{adapter}': {source}")]
    DeviceCreation {
        /// The adapter that refused.
        adapter: String,
        /// wgpu's reason.
        source: wgpu::RequestDeviceError,
    },
    /// The window handle could not become a surface.
    #[error("surface creation failed: {source}")]
    SurfaceCreation {
        /// wgpu's reason.
        source: wgpu::CreateSurfaceError,
    },
    /// The chosen adapter cannot present to the given surface at all.
    #[error("adapter '{adapter}' offers no format for this surface")]
    SurfaceUnsupported {
        /// The adapter that cannot present.
        adapter: String,
    },
    /// An upload would push resident resources past the stated budget
    /// (`Options::max_resource_bytes`). Nothing was stored.
    #[error(
        "uploading would hold {needed} resource bytes ({in_use} already resident), over the stated budget of {budget}"
    )]
    ResourceBudgetExceeded {
        /// Bytes that would be resident after the upload.
        needed: u64,
        /// Bytes resident before it.
        in_use: u64,
        /// The configured budget.
        budget: u64,
    },
    /// An upload's content violated its contract (§4.7 of the brief: refused loudly,
    /// never repaired).
    #[error("upload refused: {reason}")]
    InvalidResource {
        /// What exactly was wrong.
        reason: ResourceProblem,
    },
    /// A §7.10.5 program this device will not execute (ADR 0053). Its own variant
    /// rather than a [`ResourceProblem`] because the questions are of a different kind:
    /// a ramp is refused for what its numbers *are*, a program for what it would *do*,
    /// and the answer reaches the caller before it has built a scene at all.
    #[error("function program refused: {reason}")]
    InvalidFunction {
        /// Which of the upload's three questions was answered no, and by what.
        reason: FunctionProblem,
    },
    /// This device has issued every resource identifier it has. Nothing was stored.
    ///
    /// The identifier space is a `u32` shared by the four resource families and an id
    /// is **never** reused, so a store that has issued `u32::MAX` of them cannot admit
    /// another. Refusing is the only honest answer: reusing an id would silently replace
    /// whatever still held it, and a retained encode naming that id would draw the wrong
    /// resource with every generation counter still agreeing that nothing had moved
    /// (ADR 0048's key, ADR 0050's audit of it). Releasing resources returns bytes to
    /// the budget but never returns identifiers.
    #[error("this device has issued all {limit} resource identifiers; none can be reused")]
    ResourceIdsExhausted {
        /// The size of the identifier space, which is also how many were issued.
        limit: u32,
    },
    /// A release of a resource this device never issued or already released. An error
    /// rather than a no-op: a double release is a caller bug, and hiding it would
    /// hide the defect.
    #[error("resource {id:?} is not resident on this device")]
    UnknownResource {
        /// The identifier that was presented.
        id: quorra_scene::ResourceId,
    },
}

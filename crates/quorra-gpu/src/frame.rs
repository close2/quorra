//! The frame, and the measurements that make it accountable.
//!
//! **Skeleton — M1 fills this** (`doc/adr/0003`).
//!
//! # Why a library that is a black box is unusable here
//!
//! The caller gates performance in CI and attributes regressions by measurement. §8 is
//! blunt about what that requires of us, and §6.1 exists *because* the current backend
//! could not answer it: the readback could not be separated from the execution, so a
//! bytes-per-second estimate had to stand in for what a timestamp query would have told
//! them exactly.
//!
//! # Planned signatures
//!
//! ```text
//! pub struct Frame { /* … */ }
//!
//! impl Frame {
//!     pub fn reports(&self) -> &[Report];   // crate::report
//!     pub fn timings(&self) -> Timings;
//!     pub fn counters(&self) -> Counters;
//!     /// `Readback` targets only. `#[must_use]`, and named for what it costs.
//!     pub fn into_raster(self) -> Result<Raster, RenderError>;
//! }
//!
//! pub struct Timings {
//!     pub encode: Duration,    // CPU: turning a Scene into device commands
//!     pub upload: Duration,    // CPU->GPU transfers this frame
//!     pub execute: Duration,   // device time, from timestamp queries where available
//!     pub readback: Duration,  // GPU->CPU; zero for Surface and Texture targets
//!     pub phases: Vec<(&'static str, Duration)>,   // per-pass, when timestamps exist
//! }
//!
//! pub struct Counters {
//!     pub commands: u32,
//!     pub distinct_outlines: u32,
//!     pub atlas_entries: u32,
//!     /// §6.3: **not** the hit rate. A clip-mask cache answered all 303 lookups a page
//!     /// made and built 303 identical page-wide masks, because the key was a name rather
//!     /// than the region. A hit rate is a statement about the lookups you made, never
//!     /// about the ones you should have made.
//!     pub atlas_distinct_keys: u32,
//!     pub tiles: u32,
//!     pub segments: u32,
//!     pub bytes_uploaded: u64,
//! }
//! ```
//!
//! # The rule this module exists to enforce
//!
//! **A failed frame must not be reported as a drawn one.** Whatever a `Frame` says about
//! itself must be true — including that a `Timings` whose `execute` is a wall clock rather
//! than a timestamp query must say which it is, because a number whose provenance is
//! ambiguous cannot gate anything. Wall clocks lie under load: the mean of ten frames put
//! one of §6.1's figures at 15 ms where the fastest of ten put it at 12.
//!
//! # Also true, and easy to forget
//!
//! A `Frame` over an empty scene is a legitimate frame. It reports zero commands, no
//! reports, and `Ok`.

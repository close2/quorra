//! The WGSL this crate compiles, named once.
//!
//! A shader source reached the pipeline store through an `include_str!` at the line
//! that compiled it, which was fine while nothing else needed one. The uniform-layout
//! gate does: it reads the same source the adapter reads and derives what offset each
//! field of a `Params` sits at (`layout`, and the `#[cfg(test)]` modules beside every
//! writer of a uniform). Two `include_str!` lists of the same files would be two lists
//! that can drift, and a gate reading a file the device no longer compiles is a gate
//! that proves nothing — so there is one list, and it is this.
//!
//! What each shader *does* is stated in its own header. `function_ops.wgsl` is not here
//! because it is not a shader: it is the operator library a generated program is built
//! from, and `crate::function::OPERATORS` publishes it for the conformance corpus.

#[cfg(test)]
pub(crate) mod layout;

/// The analytic rectangle lane (§0 of the brief's second fast path).
pub(crate) const RECT: &str = include_str!("shaders/rect.wgsl");
/// The coverage-quad lane, which draws a glyph tile or a rasterised shape.
pub(crate) const COVERAGE: &str = include_str!("shaders/coverage.wgsl");
/// The image quad (ISO 32000-2 §8.9.5).
pub(crate) const IMAGE: &str = include_str!("shaders/image.wgsl");
/// The shading quad — a sampled ramp or a rasterised mesh (§8.7.4.5).
pub(crate) const SHADING: &str = include_str!("shaders/shading.wgsl");
/// Group composition: §11.3.5's blend modes and §11.4's transparency model.
pub(crate) const COMPOSITE: &str = include_str!("shaders/composite.wgsl");
/// A soft mask's reduction to one channel (§11.5).
pub(crate) const REDUCE: &str = include_str!("shaders/reduce.wgsl");
/// Pixels moved and not changed (ADR 0038, ADR 0039).
pub(crate) const BLIT: &str = include_str!("shaders/blit.wgsl");
/// One finished layer put on the surface under an affine and a filter (ADR 0056).
pub(crate) const PRESENT: &str = include_str!("shaders/present.wgsl");
/// The GPU coverage lane's winding accumulation and resolve (ADR 0016).
pub(crate) const WINDING: &str = include_str!("shaders/winding.wgsl");
/// The fixed half of a generated function paint's pipeline (ADR 0053); the program's
/// own body is appended to it.
pub(crate) const FUNCTION_LANE: &str = include_str!("shaders/function_lane.wgsl");

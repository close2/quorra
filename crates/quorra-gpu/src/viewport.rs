//! Where a scene lands: size, device transform, and what changed.
//!
//! A viewport is everything about the target that a scene is forbidden to know (§2.3
//! of the brief). One scene renders at many viewports: **zoom, scroll, window resize
//! and tiled output are all the same scene at a different viewport**, which is what
//! makes smooth zoom possible at all and what keeps 1.1–1.6 ms of encoding per frame
//! off the caller's interpreter thread.

use quorra_scene::{Affine, Rect};

/// The target-side half of a render call: size in pixels, the scene-to-pixels map, and
/// what is known to have changed.
///
/// # Two things this type is not
///
/// - **Not a page fitter.** How a fractional page becomes a whole number of pixels is
///   the caller's decision — its `TargetSpec::for_page` owns the rounding rule and its
///   pixel budget. We take a size and a transform and honour them exactly.
///   (Integration note 2 in `doc/PLAN.md`: the affine here is more general than the
///   scale their trait carries, and the bridging belongs on their side.)
/// - **Not a hint.** [`damage`] empty means "all of it", and a non-empty `damage` that
///   omitted a region which in fact changed would produce a frame that is stale in a
///   way nothing downstream can detect. Since M8 a damage list against a
///   [`Texture`] target is honoured exactly — the device touches no pixel outside
///   it (ADR 0012 carries the mechanism). A `Surface` or `Readback` target has no
///   retained contents to patch, so the device redraws everything and says so in a
///   [`Report`] — never quietly.
///
/// [`Texture`]: crate::target::Target::Texture
///
/// [`damage`]: Viewport::damage
/// [`Report`]: crate::report::Report
#[derive(Debug, Clone, Copy)]
pub struct Viewport<'a> {
    /// Target width in pixels. Zero is legitimate for a [`Readback`] target — a
    /// zero-size raster follows from a zero-size window — and refused for the others,
    /// which cannot exist at zero size.
    ///
    /// [`Readback`]: crate::target::Target::Readback
    pub width: u32,
    /// Target height in pixels. See [`width`](Viewport::width).
    pub height: u32,
    /// Maps the scene's coordinate space to target pixels. Carries the scale, **the y
    /// flip** and any tile offset — the page's own space is y-up, and the flip lives
    /// here rather than in the scene (§3 of the brief).
    pub transform: Affine,
    /// Rows or regions known to have changed, in target pixels. Empty means all of it.
    pub damage: &'a [Rect],
}

impl Viewport<'static> {
    /// A viewport with no damage list: the whole target.
    #[must_use]
    pub const fn full(width: u32, height: u32, transform: Affine) -> Self {
        Self {
            width,
            height,
            transform,
            damage: &[],
        }
    }
}

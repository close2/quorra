//! Where a scene lands: size, device transform, and what changed.
//!
//! **Skeleton — M1 fills this** (`doc/adr/0003`).
//!
//! # The contract
//!
//! A viewport is everything about the target that a scene is forbidden to know (§2.3). One
//! scene renders at many viewports: **zoom, scroll, window resize and tiled output are all
//! the same scene at a different viewport**, which is what makes smooth zoom possible at
//! all and what keeps 1.1–1.6 ms of encoding per frame off the caller's interpreter thread.
//!
//! # Planned signatures
//!
//! ```text
//! pub struct Viewport<'a> {
//!     pub width: u32,
//!     pub height: u32,
//!     /// Maps the scene's coordinate space to target pixels. Carries the scale, **the y
//!     /// flip** and any tile offset — the page's own space is y-up, and the flip lives
//!     /// here rather than in the scene (§3).
//!     pub transform: Affine,
//!     /// §6.5. Rows or regions known to have changed; empty means all of it. A caret
//!     /// blink, a selection change or a hover highlight should redraw a few tiles, not a
//!     /// page.
//!     pub damage: &'a [Rect],
//! }
//! ```
//!
//! # Two things this type is not
//!
//! - **Not a page fitter.** How a fractional page becomes a whole number of pixels is the
//!   caller's decision — its `TargetSpec::for_page` owns the rounding rule and its pixel
//!   budget. We take a size and a transform and honour them exactly. (Integration note 2 in
//!   `doc/PLAN.md`: the affine here is more general than the scale their trait carries, and
//!   the bridging belongs on their side.)
//! - **Not a hint.** `damage` empty means "all of it", and a non-empty `damage` that
//!   omitted a region which in fact changed would produce a frame that is stale in a way
//!   nothing downstream can detect. If we cannot honour the damage we were given, we redraw
//!   everything and say so in a [`Report`] — never quietly.
//!
//! [`Report`]: crate::report

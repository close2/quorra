//! Images: decoded pixels, and a filtering decision that is not ours.
//!
//! **Skeleton — M7 fills this** (`doc/adr/0003`).
//!
//! # The contract
//!
//! Decoded **RGBA8, straight alpha, row-major, no padding**. Decoding, colour conversion
//! and the choice of resampling all happen upstream; §4.5 lists two of those choices among
//! the four decisions that are the caller's and not ours:
//!
//! | decision | clause | what we do |
//! |---|---|---|
//! | `/Interpolate` | §8.9.5.3 | honour it; do not choose a filter ourselves |
//! | area averaging | a documented departure from §10.7.4 | honour it |
//!
//! **The flags are not flags.** In the caller's tree these are *methods taking the
//! placement transform* — `is_smoothed(placement)` and `area_averaged(placement) ->
//! Option<Image>` — because whether an image is smoothed depends on how far it is being
//! scaled. So what must reach us is the **resolved** decision for the placement the
//! command carries. An `ImageSpec` that carried `/Interpolate` instead would be us
//! re-deciding the question, on less information than the caller had. This is integration
//! note 1 in `doc/PLAN.md`, and it is settled before M2 freezes the API.
//!
//! # Planned signatures
//!
//! ```text
//! pub struct ImageSpec<'a> {
//!     pub width: u32,
//!     pub height: u32,
//!     /// Straight-alpha RGBA8, `width * height * 4` bytes, no row padding.
//!     pub data: &'a [u8],
//!     /// The resolved answer, not the flag it came from.
//!     pub filter: Filter,
//! }
//!
//! pub enum Filter {
//!     /// Nearest neighbour: what §8.9.5.3 asks for when `/Interpolate` is false and the
//!     /// image is being magnified.
//!     Nearest,
//!     /// Bilinear.
//!     Smooth,
//!     /// Already area-averaged by the caller for this placement; sample it directly.
//!     Prepared,
//! }
//! ```
//!
//! # Refusals, not approximations
//!
//! A 60 000 × 60 000 image is a page that exists. §5 and principle 3 agree on the
//! answer: check the allocation against a stated budget and return an error that names the
//! limit. Downsampling it quietly to fit would be a plausible-looking wrong page, and
//! deciding the filter is not ours anyway.

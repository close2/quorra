//! The fixtures more than one test file in this suite builds.
//!
//! A fixture with two copies is two fixtures: they drift, and the day one of them stops
//! reproducing the condition its test is about, that test goes on passing. This module is
//! the one home for the ones that were duplicated, and its rule is that **what moved here
//! is what each caller already built** — a fixture generalised on the way in would change
//! what somebody's assertion means without anybody deciding to.
//!
//! Five parts, along what each is about:
//!
//! - [`headless`] — the device this suite renders through, and the pixels it hands back;
//! - [`scene`] — the scene pieces more than one file draws;
//! - [`retained`] — the two pages and the render helper the `retained_*.rs` family shares,
//!   as `tests/function_support/` is shared by the `function_*.rs` family;
//! - [`clause`] — §11.4.6's line, which four files measure a frame against;
//! - [`probe`] — a drawn raster turned into the number an assertion is written on.
//!
//! The last two were listed in `doc/HANDOVER.md` as deliberately *not* unified, and the
//! recorded obstacle was the same sentence for both: each copy indexed a raster through
//! its own file's `SIZE`, so one home for them looked like one home for `SIZE`. Read
//! rather than quoted forward, that obstacle was two different mistakes. [`clause`]'s
//! arithmetic runs over every pixel of the rasters it is handed and needs **no** dimension
//! at all. [`probe`]'s needs the raster's **stride** — which is an argument, not a
//! suite-wide constant, and three files here were already passing it before the merge.
//! Neither module can read another file's `SIZE`, because neither reads a `SIZE`.
//!
//! What each caller still owns is its **expectation and its bound**: `clause`'s callers
//! each state their own partial-pixel floor, and `probe`'s each state their own tolerance.
//! A measurement is shared; a claim about a fixture is not.

#![allow(
    dead_code,
    unreachable_pub,
    reason = "each test binary compiles this module whole and uses the part it needs, so \
              every item is unused somewhere and none is reachable from outside the test \
              crate; the alternative is one copy of the fixture per test file, which is \
              what this module exists to end"
)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects,
    reason = "the test-file lint policy stated in m1.rs, restated here because a shared \
              module is compiled into binaries that do not all state it themselves"
)]

pub mod clause;
pub mod headless;
pub mod probe;
pub mod retained;
pub mod scene;

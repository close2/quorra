//! The fixtures more than one test file in this suite builds.
//!
//! A fixture with two copies is two fixtures: they drift, and the day one of them stops
//! reproducing the condition its test is about, that test goes on passing. This module is
//! the one home for the ones that were duplicated, and its rule is that **what moved here
//! is what each caller already built** — a fixture generalised on the way in would change
//! what somebody's assertion means without anybody deciding to.
//!
//! Two parts, along what each is about:
//!
//! - [`headless`] — the device this suite renders through, and the pixels it hands back;
//! - [`retained`] — the two pages and the render helper the `retained_*.rs` family shares,
//!   as `tests/function_support/` is shared by the `function_*.rs` family.

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

pub mod headless;
pub mod retained;

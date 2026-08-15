//! Shared fixtures for the §7.10.5 function tests.
//!
//! Two parts, and the split is the point: [`programs`] is *what* is evaluated and
//! [`reference`] is an evaluation written independently of the code under test, so a
//! comparison between them is evidence rather than a tautology (ADR 0037's position).

#![allow(
    dead_code,
    unreachable_pub,
    reason = "each test binary compiles this module whole and uses the part it needs, so \
              every item is unused somewhere and none is reachable from outside the test \
              crate; the alternative is one copy of the corpus per test file"
)]

pub mod lowered;
pub mod programs;
pub mod reference;

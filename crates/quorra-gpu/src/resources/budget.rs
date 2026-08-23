//! The one number every resident resource is admitted against.
//!
//! Principle 3's rule in one place: *a GPU buffer sized from document-derived arithmetic
//! is a decompression bomb with a different name*, so nothing derived from a document's
//! content is allocated without being counted against a ceiling the host stated, and
//! exceeding it is a refusal that names the limit.
//!
//! # Why this is its own module now
//!
//! `resources.rs` argued against a seam here, and its first reason was checkable:
//! `charge` had **no caller anywhere but the five `upload_*` methods**, so cutting it out
//! would have separated a private helper from every call site it had. ADR 0075 ended
//! that: an outline's quadratic form is converted on the first frame that reads it, and
//! that conversion charges the same budget from a **shared** reference, in a frame,
//! refusing in a frame's vocabulary. The counter now has two charging sites in two
//! vocabularies, which is what makes it a subject rather than a helper.
//!
//! # Why the counter is atomic when nothing charges it concurrently
//!
//! `&mut ResourceStore` — every upload and every release — and `&ResourceStore` — the
//! encoder's whole walk — are exclusive by the borrow checker, so an upload cannot run
//! while a frame converts. Within a frame, the conversion is read from the walk's thread
//! only: ADR 0054's fan-out hands each thread an `encode::parallel::Job` holding
//! `&[Segment]` and nothing else, so no worker ever reaches a `StoredOutline`.
//!
//! That makes a [`std::cell::Cell`] sufficient *today*, and it is not what is used, for
//! two reasons. It would make `Device` `!Sync`, which is a public property no test states
//! and no caller asked us to withdraw. And a counter whose correctness rests on "no other
//! thread reaches this" is a counter that the refactor which does breaks silently — where
//! an [`AtomicU64`] is right either way for the cost of one uncontended
//! compare-exchange per upload, against a `HashMap` insert on the same line.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::DeviceError;

/// A charge the budget would not take: what it would have come to, what was resident,
/// and the ceiling.
///
/// Its own type rather than a [`DeviceError`] because the budget is charged from two
/// places that refuse in two vocabularies — an upload refuses with a [`DeviceError`] and
/// a frame with a [`RenderError`](crate::error::RenderError) — and a value that carries
/// the three numbers lets each say so in its own words (ADR 0075).
#[derive(Debug, Clone, Copy)]
pub(crate) struct BudgetOverflow {
    /// What the total would have come to.
    pub needed: u64,
    /// What was resident when the charge was refused.
    pub in_use: u64,
    /// The ceiling, from
    /// [`Options::max_resource_bytes`](crate::startup::Options::max_resource_bytes).
    pub limit: u64,
}

impl From<BudgetOverflow> for DeviceError {
    fn from(over: BudgetOverflow) -> Self {
        Self::ResourceBudgetExceeded {
            needed: over.needed,
            in_use: over.in_use,
            budget: over.limit,
        }
    }
}

/// Bytes of resident, scene-derived data, against the ceiling the host stated.
#[derive(Debug)]
pub(crate) struct ResourceBudget {
    in_use: AtomicU64,
    limit: u64,
}

impl ResourceBudget {
    /// A budget with nothing resident and the host's ceiling.
    pub(crate) fn new(limit: u64) -> Self {
        Self {
            in_use: AtomicU64::new(0),
            limit,
        }
    }

    /// Bytes currently resident, for `Limits` and diagnostics.
    pub(crate) fn in_use(&self) -> u64 {
        self.in_use.load(Ordering::Relaxed)
    }

    /// Count, then admit: nothing is charged unless the total fits.
    ///
    /// The loop is the standard read-modify-write, and it exists for the reason the
    /// module comment gives rather than for a contention this crate can produce today: a
    /// `fetch_add` followed by a check would transiently over-count and could refuse a
    /// charge that fits, which is a budget that lies in the direction nobody would
    /// diagnose.
    pub(crate) fn charge(&self, bytes: u64) -> Result<(), BudgetOverflow> {
        let mut in_use = self.in_use.load(Ordering::Relaxed);
        loop {
            let needed = in_use.saturating_add(bytes);
            if needed > self.limit {
                return Err(BudgetOverflow {
                    needed,
                    in_use,
                    limit: self.limit,
                });
            }
            match self.in_use.compare_exchange_weak(
                in_use,
                needed,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Ok(()),
                Err(actual) => in_use = actual,
            }
        }
    }

    /// Return bytes that are no longer resident — a release, or a conversion another
    /// thread had already stored.
    ///
    /// Saturating, as the store's own subtraction was: a refund larger than what is
    /// resident is arithmetic that has already gone wrong, and wrapping to `u64::MAX`
    /// would turn it into a device that refuses everything for the rest of its life.
    pub(crate) fn refund(&self, bytes: u64) {
        let mut in_use = self.in_use.load(Ordering::Relaxed);
        loop {
            let left = in_use.saturating_sub(bytes);
            match self.in_use.compare_exchange_weak(
                in_use,
                left,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(actual) => in_use = actual,
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)] // test-file policy, as in `resources.rs`
mod tests {
    use super::{BudgetOverflow, ResourceBudget};
    use crate::error::DeviceError;

    /// The ceiling is a refusal that names all three numbers, and a refused charge
    /// leaves the counter where it was.
    #[test]
    fn a_refused_charge_names_its_numbers_and_costs_nothing() {
        let budget = ResourceBudget::new(100);
        budget.charge(60).expect("sixty fits under a hundred");
        let over = budget.charge(50).expect_err("sixty and fifty do not");
        assert_eq!((over.needed, over.in_use, over.limit), (110, 60, 100));
        assert_eq!(budget.in_use(), 60, "a refused charge must not charge");
    }

    /// Exactly the ceiling is admitted: the bound is `needed > limit`, so a budget
    /// filled to the byte is full rather than over.
    #[test]
    fn the_ceiling_itself_fits() {
        let budget = ResourceBudget::new(100);
        budget.charge(100).expect("a hundred fits a hundred");
        assert_eq!(budget.in_use(), 100);
        assert!(budget.charge(1).is_err());
    }

    /// A refund returns what a release or a lost conversion race gave back, and cannot
    /// wrap below zero.
    #[test]
    fn a_refund_saturates_rather_than_wrapping() {
        let budget = ResourceBudget::new(100);
        budget.charge(40).expect("forty fits");
        budget.refund(40);
        assert_eq!(budget.in_use(), 0);
        budget.refund(u64::MAX);
        assert_eq!(budget.in_use(), 0, "a refund cannot make a device unusable");
    }

    /// The overflow carries into the upload boundary's own vocabulary unchanged, which
    /// is what keeps `DeviceError::ResourceBudgetExceeded` the same three numbers it
    /// has always been.
    #[test]
    fn an_overflow_becomes_the_upload_boundarys_error() {
        let over = BudgetOverflow {
            needed: 9,
            in_use: 4,
            limit: 8,
        };
        match DeviceError::from(over) {
            DeviceError::ResourceBudgetExceeded {
                needed,
                in_use,
                budget,
            } => assert_eq!((needed, in_use, budget), (9, 4, 8)),
            other => panic!("expected ResourceBudgetExceeded, got {other:?}"),
        }
    }
}

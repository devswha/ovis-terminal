//! Budgets reserved before any buffer is allocated and released when the hold is dropped.
//!
//! One type serves every aggregate: bytes for input, output and event retention, slots for
//! queued control requests. A release wakes every task waiting for room.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::Notify;

use super::error::{BudgetScope, SchedulerError};

/// An aggregate bound shared by every holder of a reservation against it.
#[derive(Debug)]
pub struct Budget {
    scope: BudgetScope,
    limit: u64,
    held: AtomicU64,
    released: Notify,
}

impl Budget {
    /// A budget of `limit` units for `scope`.
    #[must_use]
    pub fn new(scope: BudgetScope, limit: u64) -> Arc<Self> {
        Arc::new(Self {
            scope,
            limit,
            held: AtomicU64::new(0),
            released: Notify::new(),
        })
    }

    /// Reserves `units`, or refuses without holding anything.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::Exhausted`] when the reservation would pass the limit.
    pub fn reserve(self: &Arc<Self>, units: u64) -> Result<Reservation, SchedulerError> {
        let refused = |held| SchedulerError::Exhausted {
            scope: self.scope,
            offered: units,
            held,
            limit: self.limit,
        };
        let mut held = self.held.load(Ordering::Acquire);
        loop {
            let total = held
                .checked_add(units)
                .filter(|total| *total <= self.limit)
                .ok_or_else(|| refused(held))?;
            match self
                .held
                .compare_exchange_weak(held, total, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => break,
                Err(current) => held = current,
            }
        }
        Ok(Reservation {
            budget: Arc::clone(self),
            units,
        })
    }

    /// The aggregate this budget bounds.
    #[must_use]
    pub const fn scope(&self) -> BudgetScope {
        self.scope
    }

    /// Units currently reserved.
    #[must_use]
    pub fn held(&self) -> u64 {
        self.held.load(Ordering::Acquire)
    }

    /// Resolves after the next release; create it before checking for room.
    pub(super) fn released(&self) -> tokio::sync::futures::Notified<'_> {
        self.released.notified()
    }

    fn release(&self, units: u64) {
        if units > 0 {
            self.held.fetch_sub(units, Ordering::AcqRel);
            self.released.notify_waiters();
        }
    }
}

/// Units held against a budget until this value is dropped.
#[derive(Debug)]
pub struct Reservation {
    budget: Arc<Budget>,
    units: u64,
}

impl Reservation {
    /// Units held.
    #[must_use]
    pub const fn units(&self) -> u64 {
        self.units
    }

    /// Takes over `other`'s units; both must hold the same budget.
    pub(super) fn merge(&mut self, other: Self) {
        if Arc::ptr_eq(&self.budget, &other.budget) {
            self.units = self.units.saturating_add(other.units);
            let mut other = other;
            other.units = 0;
        }
    }

    /// Returns `units` (at most all held) to the budget now.
    pub(super) fn shrink(&mut self, units: u64) {
        let units = units.min(self.units);
        self.units = self.units.saturating_sub(units);
        self.budget.release(units);
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.release(self.units);
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{Budget, BudgetScope, Reservation, SchedulerError};

    #[test]
    fn a_refusal_holds_nothing_and_a_drop_returns_the_units() {
        let budget = Budget::new(BudgetScope::TerminalInput, 10);
        let held = budget.reserve(7).expect("fits");
        let refused = budget.reserve(4).expect_err("passes the limit");
        assert_eq!(refused.kind(), "input_overflow");
        assert!(matches!(
            refused,
            SchedulerError::Exhausted {
                offered: 4,
                held: 7,
                limit: 10,
                ..
            }
        ));
        assert_eq!(budget.held(), 7);
        drop(held);
        assert_eq!(budget.held(), 0);
        assert_eq!(budget.reserve(10).map(|held| held.units()).ok(), Some(10));
        assert!(budget.reserve(u64::MAX).is_err(), "overflowing sums refuse");
    }

    #[test]
    fn merged_and_shrunk_holds_return_exactly_what_they_hold() {
        let budget = Budget::new(BudgetScope::SessionOutput, 100);
        let mut held = budget.reserve(30).expect("fits");
        held.merge(budget.reserve(20).expect("fits"));
        assert_eq!((held.units(), budget.held()), (50, 50));
        held.shrink(45);
        assert_eq!((held.units(), budget.held()), (5, 5));
        held.shrink(99);
        assert_eq!((held.units(), budget.held()), (0, 0));
        let other = Budget::new(BudgetScope::SessionOutput, 100);
        held.merge(other.reserve(9).expect("fits"));
        assert_eq!(
            (held.units(), other.held()),
            (0, 0),
            "a foreign hold is released"
        );
    }

    #[tokio::test]
    async fn a_release_wakes_a_waiter_registered_before_it() {
        let budget = Budget::new(BudgetScope::SessionControl, 1);
        let held = budget.reserve(1).expect("fits");
        let released = budget.released();
        drop(held);
        released.await;
        assert_eq!(budget.reserve(1).map(|held| held.units()).ok(), Some(1));
    }

    proptest! {
        #[test]
        fn held_units_never_pass_the_limit(
            limit in 0_u64..1_000,
            asks in proptest::collection::vec((0_u64..400, any::<bool>()), 1..60),
        ) {
            let budget = Budget::new(BudgetScope::SessionInput, limit);
            let mut kept = Vec::new();
            for (units, keep) in asks {
                let before = budget.held();
                match budget.reserve(units) {
                    Ok(reservation) if keep => kept.push(reservation),
                    Ok(reservation) => drop(reservation),
                    Err(_) => prop_assert_eq!(budget.held(), before),
                }
                prop_assert!(budget.held() <= limit);
                prop_assert_eq!(budget.held(), kept.iter().map(Reservation::units).sum::<u64>());
            }
        }
    }
}

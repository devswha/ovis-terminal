//! Byte budgets reserved before any buffer is allocated and released when it is dropped.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::error::{BudgetScope, SchedulerError};

/// An aggregate byte bound shared by every holder of a reservation against it.
#[derive(Debug)]
pub struct ByteBudget {
    scope: BudgetScope,
    limit: u64,
    held: AtomicU64,
}

impl ByteBudget {
    /// A budget of `limit` bytes for `scope`.
    #[must_use]
    pub fn new(scope: BudgetScope, limit: u64) -> Arc<Self> {
        Arc::new(Self {
            scope,
            limit,
            held: AtomicU64::new(0),
        })
    }

    /// Reserves `bytes`, or refuses without holding anything.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::Exhausted`] when the reservation would pass the limit.
    pub fn reserve(self: &Arc<Self>, bytes: u64) -> Result<Reservation, SchedulerError> {
        let refused = |held| SchedulerError::Exhausted {
            scope: self.scope,
            offered: bytes,
            held,
            limit: self.limit,
        };
        let mut held = self.held.load(Ordering::Acquire);
        loop {
            let total = held
                .checked_add(bytes)
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
            bytes,
        })
    }

    /// Bytes currently reserved.
    #[must_use]
    pub fn held(&self) -> u64 {
        self.held.load(Ordering::Acquire)
    }

    /// The bound.
    #[must_use]
    pub const fn limit(&self) -> u64 {
        self.limit
    }
}

/// Bytes held against a budget until this value is dropped.
#[derive(Debug)]
pub struct Reservation {
    budget: Arc<ByteBudget>,
    bytes: u64,
}

impl Reservation {
    /// Bytes held.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.held.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{BudgetScope, ByteBudget, Reservation};

    #[test]
    fn a_refusal_holds_nothing_and_a_drop_returns_the_bytes() {
        let budget = ByteBudget::new(BudgetScope::TerminalInput, 10);
        let held = budget.reserve(7).expect("fits");
        let refused = budget.reserve(4).expect_err("passes the limit");
        assert_eq!(refused.kind(), "input_overflow");
        assert!(matches!(
            refused,
            super::SchedulerError::Exhausted {
                offered: 4,
                held: 7,
                limit: 10,
                ..
            }
        ));
        assert_eq!(budget.held(), 7);
        drop(held);
        assert_eq!(budget.held(), 0);
        assert_eq!(budget.reserve(10).map(|held| held.bytes()).ok(), Some(10));
        assert!(budget.reserve(u64::MAX).is_err(), "overflowing sums refuse");
    }

    proptest! {
        #[test]
        fn held_bytes_never_pass_the_limit(
            limit in 0_u64..1_000,
            asks in proptest::collection::vec((0_u64..400, any::<bool>()), 1..60),
        ) {
            let budget = ByteBudget::new(BudgetScope::SessionInput, limit);
            let mut kept = Vec::new();
            for (bytes, keep) in asks {
                let before = budget.held();
                match budget.reserve(bytes) {
                    Ok(reservation) if keep => kept.push(reservation),
                    Ok(reservation) => drop(reservation),
                    Err(_) => prop_assert_eq!(budget.held(), before),
                }
                prop_assert!(budget.held() <= limit);
                prop_assert_eq!(budget.held(), kept.iter().map(Reservation::bytes).sum::<u64>());
            }
        }
    }
}

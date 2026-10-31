//! The per-connection bound on encoded bytes queued for the peer but not yet written.
//!
//! Every reply and notification reserves its encoded size before it is queued and
//! releases it after the writer finished writing it, so the record being written counts
//! until it left. The first refused reservation is published once; the connection then
//! closes with `$/closing{reason: backpressure}`.

use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::watch;

/// A refused reservation: the bound and the bytes already queued when it was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Overflow {
    pub(super) limit: u64,
    pub(super) queued: u64,
}

#[derive(Debug)]
pub(super) struct Budget {
    limit: u64,
    // Only this counter is shared through it; no other memory is published, so Relaxed.
    queued: AtomicU64,
    overflow: watch::Sender<Option<Overflow>>,
}

impl Budget {
    pub(super) fn new(limit: u64) -> Self {
        Self {
            limit,
            queued: AtomicU64::new(0),
            overflow: watch::Sender::new(None),
        }
    }

    /// Admits `bytes` more unwritten bytes, or refuses them without wrapping the count.
    pub(super) fn reserve(&self, bytes: u64) -> Result<(), Overflow> {
        let mut queued = self.queued.load(Ordering::Relaxed);
        loop {
            let Some(total) = queued
                .checked_add(bytes)
                .filter(|total| *total <= self.limit)
            else {
                return Err(self.refuse(queued));
            };
            let swapped = self.queued.compare_exchange_weak(
                queued,
                total,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
            match swapped {
                Ok(_) => return Ok(()),
                Err(current) => queued = current,
            }
        }
    }

    fn refuse(&self, queued: u64) -> Overflow {
        let overflow = Overflow {
            limit: self.limit,
            queued,
        };
        self.overflow.send_if_modified(|first| {
            let unset = first.is_none();
            if unset {
                *first = Some(overflow);
            }
            unset
        });
        overflow
    }

    /// Returns `bytes` the writer finished writing or discarded.
    pub(super) fn release(&self, bytes: u64) {
        self.queued.fetch_sub(bytes, Ordering::Relaxed);
    }

    /// The first refused reservation, if any.
    #[cfg(test)]
    fn overflowed(&self) -> Option<Overflow> {
        *self.overflow.borrow()
    }

    /// Notifies when the first reservation is refused.
    pub(super) fn subscribe(&self) -> watch::Receiver<Option<Overflow>> {
        self.overflow.subscribe()
    }

    /// Bytes reserved and not yet released.
    #[cfg(test)]
    fn queued(&self) -> u64 {
        self.queued.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::{Budget, Overflow};

    #[test]
    fn reservations_stop_exactly_at_the_bound() {
        let budget = Budget::new(64);
        assert_eq!(budget.reserve(60), Ok(()));
        assert_eq!(budget.reserve(4), Ok(()));
        let refused = Overflow {
            limit: 64,
            queued: 64,
        };
        assert_eq!(budget.reserve(1), Err(refused));
        assert_eq!(budget.overflowed(), Some(refused));
        budget.release(60);
        assert_eq!(budget.queued(), 4);
    }

    #[test]
    fn accounting_never_wraps_at_u64_max() {
        let empty = Budget::new(64);
        assert_eq!(
            empty.reserve(u64::MAX),
            Err(Overflow {
                limit: 64,
                queued: 0
            })
        );
        assert_eq!(empty.queued(), 0);

        let unbounded = Budget::new(u64::MAX);
        assert_eq!(unbounded.reserve(u64::MAX), Ok(()));
        let full = Overflow {
            limit: u64::MAX,
            queued: u64::MAX,
        };
        assert_eq!(unbounded.reserve(1), Err(full));
        assert_eq!(unbounded.queued(), u64::MAX);
    }

    #[test]
    fn only_the_first_refusal_is_published() {
        let budget = Budget::new(8);
        assert_eq!(budget.reserve(6), Ok(()));
        let first = budget.reserve(4).err();
        budget.release(6);
        assert_eq!(
            budget.reserve(9).err(),
            Some(Overflow {
                limit: 8,
                queued: 0
            })
        );
        assert_eq!(budget.overflowed(), first);
    }
}

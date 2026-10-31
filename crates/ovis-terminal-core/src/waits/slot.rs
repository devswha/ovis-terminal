//! One observer's outcome slot and the caller's ticket for it.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use super::entry::WatchId;
use super::error::WaitError;
use super::hub::Shared;

/// Where a feed, the deadline or a cancellation leaves the outcome; first writer wins.
#[derive(Debug)]
pub(super) struct Slot<O> {
    outcome: Mutex<Option<O>>,
    ready: Condvar,
}

impl<O> Slot<O> {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            outcome: Mutex::new(None),
            ready: Condvar::new(),
        })
    }

    /// Recovers a poisoned slot: an `Option` written in one assignment is never torn.
    fn lock(&self) -> MutexGuard<'_, Option<O>> {
        self.outcome.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn fill(&self, outcome: O) {
        let mut slot = self.lock();
        if slot.is_none() {
            *slot = Some(outcome);
        }
        drop(slot);
        self.ready.notify_all();
    }

    fn take(&self) -> Option<O> {
        self.lock().take()
    }

    fn take_by(&self, deadline: Instant) -> Option<O> {
        let mut slot = self.lock();
        loop {
            if let Some(outcome) = slot.take() {
                return Some(outcome);
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            if left.is_zero() {
                return None;
            }
            slot = self
                .ready
                .wait_timeout(slot, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// The caller's handle on one armed wait; dropping it detaches the observer.
#[derive(Debug)]
pub struct Ticket<O> {
    hub: Arc<Shared>,
    id: Option<WatchId>,
    slot: Arc<Slot<O>>,
}

impl<O> Ticket<O> {
    pub(super) const fn new(hub: Arc<Shared>, id: Option<WatchId>, slot: Arc<Slot<O>>) -> Self {
        Self { hub, id, slot }
    }

    /// Blocks until a feed resolves the wait, its scope is cancelled or `deadline` passes.
    ///
    /// The deadline outcome is the wait's typed timeout; it never reports a match.
    ///
    /// # Errors
    ///
    /// Returns [`WaitError::Poisoned`] when the wait table is poisoned.
    pub fn wait(self, deadline: Instant) -> Result<O, WaitError> {
        if let Some(outcome) = self.slot.take_by(deadline) {
            return Ok(outcome);
        }
        if let Some(id) = self.id {
            self.hub.expire(id)?;
        }
        self.slot.take().ok_or(WaitError::Detached)
    }
}

impl<O> Drop for Ticket<O> {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            self.hub.detach(id);
        }
    }
}

//! An admitted operation's identity between dispatch and its durable accept.
//!
//! The job decides how to accept while holding the phase; a cancellation that takes the
//! phase first is therefore committed with the accept, never lost to a `not_found` window.

use tokio::sync::{Mutex, MutexGuard, watch};

use super::deadline::{Reply, Response};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Reserved,
    CancelRequested,
    Accepted,
}

pub(super) struct Reservation {
    phase: Mutex<Phase>,
    cancelled: watch::Sender<Option<Reply>>,
}

impl Reservation {
    pub(super) fn new() -> Self {
        Self {
            phase: Mutex::new(Phase::Reserved),
            cancelled: watch::channel(None).0,
        }
    }

    /// Records a cancellation before the accept, or returns `None` once it has committed.
    pub(super) async fn cancel(&self) -> Option<Response> {
        let mut phase = self.phase.lock().await;
        if *phase == Phase::Accepted {
            return None;
        }
        *phase = Phase::CancelRequested;
        let delivered = self.cancelled.subscribe();
        drop(phase);
        Some(delivered)
    }

    /// Holds the phase for the accept; call only from a blocking worker.
    pub(super) fn accept(&self) -> Acceptance<'_> {
        Acceptance {
            phase: self.phase.blocking_lock(),
            cancelled: &self.cancelled,
        }
    }
}

/// The accept's exclusive view of a reservation; dropping it marks the identity accepted.
pub(super) struct Acceptance<'a> {
    phase: MutexGuard<'a, Phase>,
    cancelled: &'a watch::Sender<Option<Reply>>,
}

impl Acceptance<'_> {
    pub(super) fn cancel_requested(&self) -> bool {
        *self.phase == Phase::CancelRequested
    }

    /// Answers every waiting cancellation with the outcome the accept actually committed.
    pub(super) fn deliver(&self, reply: Reply) {
        self.cancelled.send_replace(Some(reply));
    }
}

impl Drop for Acceptance<'_> {
    fn drop(&mut self) {
        *self.phase = Phase::Accepted;
    }
}

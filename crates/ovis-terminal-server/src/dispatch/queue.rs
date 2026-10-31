//! A dispatched operation's place in the core's FIFO pinned-snapshot queue.
//!
//! The ticket is taken when the request is dispatched, so its position is inspectable
//! by `op.get` and reported on a deadline until the core grants it a slot. Each
//! connection holds at most the advertised `queued_operations` waiting tickets.

use std::sync::Arc;

use ovis_terminal_core::events::{QueueTicket, SnapshotQueue, TicketId};
use ovis_terminal_protocol::schema::ids::MethodName;
use ovis_terminal_protocol::schema::scalar::QueuePosition;
use tokio::sync::OwnedSemaphorePermit;

/// Inspects or withdraws one dispatched request's ticket.
#[derive(Debug, Clone)]
pub(super) struct Queued {
    pub queue: Arc<SnapshotQueue>,
    pub ticket: TicketId,
    pub method: MethodName,
}

impl Queued {
    /// The current one-based place in line, or `None` once the request was admitted.
    pub(super) fn position(&self) -> Option<QueuePosition> {
        self.queue.position(self.ticket)
    }

    pub(super) fn withdraw(&self) {
        self.queue.withdraw(self.ticket);
    }
}

/// The job's ticket and the connection queue permit it holds until its turn.
#[derive(Debug)]
pub(super) struct Admission {
    pub ticket: QueueTicket,
    pub permit: OwnedSemaphorePermit,
    pub queued: Queued,
}

/// Withdraws the ticket if the job's future is dropped before its blocking wait returns.
///
/// A dropped future cannot stop the blocking worker that waits for the slot; without the
/// withdrawal it would wait forever and block runtime shutdown.
#[derive(Debug)]
pub(super) struct WithdrawOnDrop(pub Queued);

impl Drop for WithdrawOnDrop {
    fn drop(&mut self) {
        self.0.withdraw();
    }
}

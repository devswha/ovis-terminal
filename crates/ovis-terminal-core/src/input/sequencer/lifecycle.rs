//! Stream readiness, holder loss and exit: what each does to queued and sent entries.

use ovis_terminal_protocol::schema::methods::server::WriteSequence;

use super::Sequencer;
use super::state::{Readiness, Sent};
use crate::input::receipts::{Delivery, FenceReason, Receipt};
use crate::registry::ProcessGeneration;

/// Sent entries whose evidence ended, kept so a late acknowledgement can still resolve them.
const UNKNOWN_KEPT: usize = 64;

impl Sequencer {
    /// The holder reports `process` running at its grid; writes continue after the holder's
    /// last acknowledged write sequence.
    ///
    /// Entries admitted for another process are fenced as `stale_target` with zero bytes.
    /// Nothing whose delivery is uncertain is sent again.
    pub fn stream_ready(
        &mut self,
        process: ProcessGeneration,
        acknowledged: WriteSequence,
    ) -> Vec<Receipt> {
        if self.readiness != Readiness::Live(process) {
            // Replies answered the previous program's queries.
            self.replies.clear();
        }
        self.readiness = Readiness::Live(process);
        self.next_write = self.next_write.max(acknowledged.get().saturating_add(1));
        let mut fenced = Vec::new();
        for entry in self.queue.drain() {
            if entry.process == process {
                self.queue.push(entry);
            } else {
                let reason = FenceReason::StaleTarget;
                fenced.extend(self.receipts.settle(entry.seq, Delivery::Fenced { reason }));
            }
        }
        fenced
    }

    /// The holder link was lost: input-ready is revoked, unsent entries are fenced and the
    /// entry with the holder becomes `outcome_unknown`.
    pub fn link_lost(&mut self) -> Vec<Receipt> {
        self.end(Readiness::Recovering, FenceReason::Recovering)
    }

    /// The program exited: unsent entries are fenced; an unacknowledged one is unknown.
    pub fn exited(&mut self) -> Vec<Receipt> {
        self.end(Readiness::Finished, FenceReason::Finished)
    }

    fn end(&mut self, readiness: Readiness, reason: FenceReason) -> Vec<Receipt> {
        self.readiness = readiness;
        self.replies.clear();
        self.handshake = None;
        let mut changed = Vec::new();
        if let Some(flight) = self.in_flight.take() {
            changed.extend(
                self.receipts
                    .settle(flight.sent.seq, Delivery::OutcomeUnknown),
            );
            self.remember(flight.sent);
        }
        for entry in self.queue.drain() {
            changed.extend(self.receipts.settle(entry.seq, Delivery::Fenced { reason }));
        }
        changed
    }

    fn remember(&mut self, sent: Sent) {
        if self.unknown.len() >= UNKNOWN_KEPT {
            self.unknown.pop_front();
        }
        self.unknown.push_back(sent);
    }
}

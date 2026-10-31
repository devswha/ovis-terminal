//! The last write boundary and the fence that invalidates unsent entries.

use ovis_terminal_holder::protocol::Input;

use super::state::{InFlight, Readiness, Sent};
use super::{Sequencer, SequencerError, View};
use crate::input::receipts::{Delivery, Receipt};

/// Receipts that changed and at most one holder write to perform, in this order.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct Step {
    /// Entries fenced with zero bytes.
    pub fenced: Vec<Receipt>,
    /// Input to hand to the holder.
    pub dispatch: Option<Input>,
}

impl Sequencer {
    /// Rechecks the head entry against the current state and hands it to the holder.
    ///
    /// Query replies go first; a user entry goes only when none is with the holder. Entries
    /// failing the recheck are fenced with zero bytes and the next one is tried. Call again
    /// after every acknowledgement until no dispatch is returned.
    ///
    /// # Errors
    ///
    /// `limit_exceeded` when the holder write sequence is exhausted.
    pub fn next(&mut self, view: &View<'_>) -> Result<Step, SequencerError> {
        let mut step = Step::default();
        let Readiness::Live(process) = self.readiness else {
            return Ok(step);
        };
        if let Some(reply) = self.replies.take() {
            let input = self.holder_write((process, 0), reply)?;
            self.replies.sent(input.sequence);
            step.dispatch = Some(input);
            return Ok(step);
        }
        if self.in_flight.is_some() {
            return Ok(step);
        }
        while let Some(entry) = self.queue.pop() {
            if let Err(reason) = entry.recheck(&self.check(view, process)) {
                step.fenced
                    .extend(self.receipts.settle(entry.seq, Delivery::Fenced { reason }));
                continue;
            }
            let input = self.holder_write(
                (process, entry.grant_epoch()),
                entry.encoded.bytes().to_vec(),
            )?;
            let sent = Sent {
                seq: entry.seq,
                write: input.sequence,
                target: input.target.clone(),
                grant: input.grant.clone(),
                len: entry.len(),
            };
            self.in_flight = Some(InFlight {
                sent,
                entry,
                fencing: None,
            });
            step.dispatch = Some(input);
            break;
        }
        Ok(step)
    }

    /// Rechecks every entry after a revocation, takeover or credential change.
    ///
    /// Unsent entries that fail are fenced now with zero bytes. When the entry with the
    /// holder fails, the fence epoch advances and an empty holder write at that epoch is
    /// returned: the holder then drops the entry if it has not started writing it, and
    /// whatever it already wrote is still reported.
    ///
    /// # Errors
    ///
    /// `limit_exceeded` when the fence epoch or write sequence is exhausted.
    pub fn fence(&mut self, view: &View<'_>) -> Result<Step, SequencerError> {
        let mut step = Step::default();
        let Readiness::Live(process) = self.readiness else {
            return Ok(step);
        };
        for entry in self.queue.drain() {
            match entry.recheck(&self.check(view, process)) {
                Ok(()) => self.queue.push(entry),
                Err(reason) => step
                    .fenced
                    .extend(self.receipts.settle(entry.seq, Delivery::Fenced { reason })),
            }
        }
        let failed = self
            .in_flight
            .as_ref()
            .filter(|flight| flight.fencing.is_none())
            .and_then(|flight| flight.entry.recheck(&self.check(view, process)).err());
        if let (Some(reason), Some(flight)) = (failed, self.in_flight.as_mut()) {
            flight.fencing = Some(reason);
            self.fence_epoch = self
                .fence_epoch
                .checked_add(1)
                .ok_or(SequencerError::Exhausted)?;
            let input = self.holder_write((process, 0), Vec::new())?;
            self.handshake = Some(input.sequence);
            step.dispatch = Some(input);
        }
        Ok(step)
    }
}

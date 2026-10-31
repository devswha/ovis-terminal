//! Holder evidence: acknowledged write prefixes and refusals before queueing.

use ovis_terminal_holder::protocol::InputWritten;
use ovis_terminal_protocol::schema::methods::server::WriteSequence;
use ovis_terminal_protocol::schema::scalar::{ByteCount, ByteOffset};

use super::state::Sent;
use super::{Sequencer, SequencerError};
use crate::input::receipts::{Delivery, FenceReason, Receipt};

impl Sent {
    fn matches(&self, ack: &InputWritten) -> bool {
        ack.target == self.target && ack.grant_epoch == self.grant.grant_epoch
    }

    /// The delivery the holder's acknowledgement proves; a zero-byte write is a fence.
    fn delivery(&self, ack: &InputWritten, fencing: Option<FenceReason>) -> Delivery {
        let length = u64::try_from(self.len).unwrap_or(u64::MAX);
        let written_bytes = ByteCount::new(ack.written_bytes);
        let output_offset = ByteOffset::new(ack.output_offset);
        match ack.written_bytes {
            0 => Delivery::Fenced {
                reason: fencing.unwrap_or(FenceReason::Unwritten),
            },
            written if written >= length => Delivery::PtyWritten {
                written_bytes,
                output_offset,
            },
            _ => Delivery::Partial {
                written_bytes,
                output_offset,
            },
        }
    }
}

impl Sequencer {
    /// Applies the holder's acknowledgement of one write; returns the receipt it changed.
    ///
    /// An acknowledgement arriving after evidence ended still resolves `outcome_unknown`
    /// to the exact bytes written.
    ///
    /// # Errors
    ///
    /// `stale_seq` for a write this sequencer has no record of and `stale_target` for an
    /// acknowledgement naming another target or grant.
    pub fn acknowledge(&mut self, ack: &InputWritten) -> Result<Option<Receipt>, SequencerError> {
        let sequence = ack.sequence;
        if self.replies.acknowledge(sequence) {
            return Ok(None);
        }
        if self.handshake == Some(sequence) {
            self.handshake = None;
            return Ok(None);
        }
        if let Some(flight) = self
            .in_flight
            .take_if(|flight| flight.sent.write == sequence)
        {
            if !flight.sent.matches(ack) {
                self.in_flight = Some(flight);
                return Err(SequencerError::ForeignAck { sequence });
            }
            let delivery = flight.sent.delivery(ack, flight.fencing);
            return Ok(self.receipts.settle(flight.sent.seq, delivery));
        }
        let position = self
            .unknown
            .iter()
            .position(|sent| sent.write == sequence)
            .ok_or(SequencerError::UnknownAck { sequence })?;
        if self
            .unknown
            .get(position)
            .is_some_and(|sent| !sent.matches(ack))
        {
            return Err(SequencerError::ForeignAck { sequence });
        }
        Ok(self
            .unknown
            .remove(position)
            .and_then(|sent| self.receipts.settle(sent.seq, sent.delivery(ack, None))))
    }

    /// Records that the holder refused a write before queueing any of its bytes.
    pub fn refused(&mut self, sequence: WriteSequence, reason: FenceReason) -> Option<Receipt> {
        let sequence = sequence.get();
        if self.replies.acknowledge(sequence) {
            return None;
        }
        if self.handshake == Some(sequence) {
            self.handshake = None;
            return None;
        }
        let flight = self
            .in_flight
            .take_if(|flight| flight.sent.write == sequence)?;
        self.receipts
            .settle(flight.sent.seq, Delivery::Fenced { reason })
    }
}

/// The fence reason of a holder refusal; every refusal of `Holder::input` queued nothing.
#[cfg(unix)]
pub const fn holder_refusal(error: &ovis_terminal_holder::unix::HolderError) -> FenceReason {
    use ovis_terminal_holder::unix::HolderError;
    match error {
        HolderError::InputOverflow { .. } => FenceReason::InputOverflow,
        HolderError::ProcessExited => FenceReason::Finished,
        HolderError::UnknownTerminal { .. }
        | HolderError::StaleTarget { .. }
        | HolderError::SequenceRegression { .. } => FenceReason::StaleTarget,
        _ => FenceReason::Unwritten,
    }
}

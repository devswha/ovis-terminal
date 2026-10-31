//! The sequencer of one terminal incarnation, its lookups and barriers.

use std::collections::VecDeque;

use ovis_terminal_holder::protocol::{Input, ProcessIncarnation, TerminalTarget, WriteGrant};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::scalar::ByteCount;

use super::entry::{Check, Entry, Queue};
use super::replies::Replies;
use super::{Binding, QueueLimits, SequencerError, View};
use crate::input::receipts::{
    Delivery, FenceReason, InputSeq, Receipt, ReceiptBook, ReceiptError, StageReport,
};
use crate::registry::ProcessGeneration;

/// Whether the runtime can take input for the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Readiness {
    /// No program stream at a known grid yet.
    NotReady,
    /// The holder reported this process running at its grid.
    Live(ProcessGeneration),
    /// The holder link was lost.
    Recovering,
    /// The program exited.
    Finished,
}

/// An entry handed to the holder.
#[derive(Debug, Clone)]
pub(super) struct Sent {
    pub(super) seq: InputSeq,
    pub(super) write: u64,
    pub(super) target: TerminalTarget,
    pub(super) grant: WriteGrant,
    pub(super) len: usize,
}

/// The one user entry with the holder, still rechecked when authority moves.
#[derive(Debug)]
pub(super) struct InFlight {
    pub(super) sent: Sent,
    pub(super) entry: Entry,
    pub(super) fencing: Option<FenceReason>,
}

/// Where the entries up to one input position stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BarrierState {
    /// Bytes up to the position are still queued or with the holder.
    Pending {
        /// Admitted bytes not yet acknowledged.
        pending_bytes: ByteCount,
    },
    /// Every entry up to the position was written in full.
    Reached,
    /// Nothing is pending, but this entry was fenced, partly written or has no evidence.
    Undelivered {
        /// Earliest such entry.
        first: InputSeq,
    },
}

/// The ordered input sequence of one terminal incarnation.
#[derive(Debug)]
pub struct Sequencer {
    pub(super) binding: Binding,
    pub(super) limits: QueueLimits,
    pub(super) readiness: Readiness,
    pub(super) queue: Queue,
    pub(super) in_flight: Option<InFlight>,
    pub(super) unknown: VecDeque<Sent>,
    pub(super) handshake: Option<u64>,
    pub(super) replies: Replies,
    pub(super) last_input: u64,
    pub(super) next_write: u64,
    pub(super) fence_epoch: u64,
    pub(super) receipts: ReceiptBook,
}

impl Sequencer {
    /// An empty sequence that admits nothing until the stream is ready.
    pub fn new(binding: Binding, limits: QueueLimits) -> Self {
        Self {
            binding,
            limits,
            readiness: Readiness::NotReady,
            queue: Queue::default(),
            in_flight: None,
            unknown: VecDeque::new(),
            handshake: None,
            replies: Replies::default(),
            last_input: 0,
            next_write: 1,
            fence_epoch: 1,
            receipts: ReceiptBook::default(),
        }
    }

    /// The retained receipt of `op_id`.
    pub fn receipt(&self, op_id: &OpId) -> Option<&Receipt> {
        self.receipts.by_op(op_id)
    }

    /// Applies a producer's report of an agent stage.
    ///
    /// # Errors
    ///
    /// `not_found` for an unknown operation, `invalid_record` for a stage without a declared
    /// producer or an acceptance before the PTY write.
    pub fn report(&mut self, report: &StageReport) -> Result<Receipt, ReceiptError> {
        self.receipts.report(report)
    }

    /// Where the entries up to `seq` stand.
    ///
    /// # Errors
    ///
    /// `seq_ahead` when no entry was admitted at `seq`.
    pub fn barrier(&self, seq: InputSeq) -> Result<BarrierState, SequencerError> {
        if seq.get() == 0 || seq.get() > self.last_input {
            return Err(SequencerError::SeqAhead { requested: seq });
        }
        let in_flight = self
            .in_flight
            .as_ref()
            .filter(|flight| flight.sent.seq <= seq)
            .map_or(0, |flight| flight.sent.len);
        let pending = self.queue.pending_through(seq).saturating_add(in_flight);
        if pending > 0 {
            let pending_bytes = ByteCount::new(u64::try_from(pending).unwrap_or(u64::MAX));
            return Ok(BarrierState::Pending { pending_bytes });
        }
        Ok(self
            .receipts
            .through(seq)
            .find(|receipt| !matches!(receipt.delivery, Delivery::PtyWritten { .. }))
            .map_or(BarrierState::Reached, |receipt| BarrierState::Undelivered {
                first: receipt.queued_seq,
            }))
    }

    pub(super) const fn live(&self) -> Result<ProcessGeneration, SequencerError> {
        match self.readiness {
            Readiness::Live(process) => Ok(process),
            Readiness::NotReady => Err(SequencerError::NotReady),
            Readiness::Recovering => Err(SequencerError::Recovering),
            Readiness::Finished => Err(SequencerError::Finished),
        }
    }

    pub(super) fn check<'a>(&'a self, view: &'a View<'a>, process: ProcessGeneration) -> Check<'a> {
        Check {
            view,
            authority: self.binding.authority(),
            process,
        }
    }

    /// The next holder write, bound to `process`, the lease epoch and the fence epoch.
    pub(super) fn holder_write(
        &mut self,
        process: (ProcessGeneration, u64),
        bytes: Vec<u8>,
    ) -> Result<Input, SequencerError> {
        let (process, grant_epoch) = process;
        let sequence = self.next_write;
        self.next_write = sequence.checked_add(1).ok_or(SequencerError::Exhausted)?;
        Ok(Input {
            target: self.binding.holder_target(process),
            sequence,
            grant: WriteGrant {
                grant_epoch,
                fence_epoch: self.fence_epoch,
                process_incarnation: ProcessIncarnation::new(process.get()),
            },
            bytes,
        })
    }
}

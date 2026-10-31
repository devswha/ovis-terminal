//! Core requests against one held terminal: input, resize, signal, tail and forget.
//!
//! Input is checked against the process incarnation its grant names and against the
//! newest fence epoch the core announced; a stale grant is acknowledged with zero bytes
//! written, never queued. Resize and signal never wait behind queued input.

use std::sync::Arc;

use ovis_terminal_protocol::domain::conpty::ConptyGrid;

use super::{Entry, INPUT_BYTES, INPUT_ENTRIES, Pending, Table, lock};
use crate::protocol::{
    Forget, Input, InputRejected, InputRejection, InputWritten, Message, Resize, Signal,
    SignalKind, Tail, TailRequest, TerminalTarget,
};
use crate::service::windows::link::LinkError;
use crate::windows::ConptyError;
use crate::windows::model::{InputRejected as Refused, OutputOffset, OutputRead};

/// The byte a console reads as Ctrl+C; ConPTY raises `CTRL_C_EVENT` for it.
const INTERRUPT: u8 = 0x03;

fn count(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

impl Table {
    fn entry(&self, target: &TerminalTarget) -> Result<Arc<Entry>, LinkError> {
        let entry = lock(&self.entries)
            .get(&target.terminal_id)
            .cloned()
            .ok_or_else(|| LinkError::UnknownTerminal {
                terminal_id: target.terminal_id.clone(),
            })?;
        let process_matches = target
            .process_incarnation
            .is_none_or(|process| entry.target.process_incarnation == Some(process));
        if entry.target.terminal_incarnation != target.terminal_incarnation || !process_matches {
            return Err(LinkError::StaleTarget {
                terminal_id: target.terminal_id.clone(),
            });
        }
        Ok(entry)
    }

    fn unwritten(&self, entry: &Entry, input: &Input) {
        let (output_offset, fence_epoch) = {
            let evidence = lock(&entry.evidence);
            (evidence.output, evidence.fence)
        };
        self.publish(Message::InputWritten(InputWritten {
            target: entry.target.clone(),
            sequence: input.sequence,
            grant_epoch: input.grant.grant_epoch,
            fence_epoch,
            ack_epoch: self.next_ack(),
            written_bytes: 0,
            output_offset,
        }));
    }

    /// Queues `input` whole, or answers why none of it was queued.
    pub(in crate::service::windows) fn input(&self, input: Input) -> Result<(), LinkError> {
        let entry = self.entry(&input.target)?;
        if entry.target.process_incarnation != Some(input.grant.process_incarnation) {
            return Err(LinkError::StaleTarget {
                terminal_id: input.target.terminal_id,
            });
        }
        let mut evidence = lock(&entry.evidence);
        let fenced = input.grant.fence_epoch < evidence.fence;
        evidence.fence = evidence.fence.max(input.grant.fence_epoch);
        let fence_epoch = evidence.fence;
        if fenced || input.bytes.is_empty() {
            drop(evidence);
            self.unwritten(&entry, &input);
            return Ok(());
        }
        let bytes = count(input.bytes.len());
        match entry.terminal.input(input.bytes.clone()) {
            Ok(queued) => {
                evidence.pending.push_back(Pending {
                    queued,
                    sequence: input.sequence,
                    grant_epoch: input.grant.grant_epoch,
                    fence_epoch,
                    bytes,
                });
                Ok(())
            }
            Err(Refused::Overflow { queued, .. }) => {
                drop(evidence);
                self.publish(Message::InputRejected(InputRejected {
                    target: entry.target.clone(),
                    sequence: input.sequence,
                    reason: InputRejection::InputOverflow,
                    hint: "wait for earlier input to be written, then resend".to_owned(),
                    queued_bytes: count(queued.bytes),
                    queued_entries: count(queued.entries),
                    limit_bytes: count(INPUT_BYTES),
                    limit_entries: count(INPUT_ENTRIES),
                }));
                Ok(())
            }
            Err(Refused::Closed) => {
                drop(evidence);
                self.unwritten(&entry, &input);
                Ok(())
            }
            Err(Refused::Exhausted) => Err(LinkError::CounterExhausted),
        }
    }

    /// Resizes the pseudoconsole; an exited program has no console left to resize.
    pub(in crate::service::windows) fn resize(&self, resize: &Resize) -> Result<(), LinkError> {
        let entry = self.entry(&resize.target)?;
        let (cols, rows) = resize.geometry;
        let grid = ConptyGrid::new(cols, rows).map_err(ConptyError::from)?;
        match entry.terminal.resize(grid) {
            Ok(()) | Err(ConptyError::ProcessExited) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Interrupts with Ctrl+C, or ends the whole process tree.
    pub(in crate::service::windows) fn signal(&self, signal: &Signal) -> Result<(), LinkError> {
        let entry = self.entry(&signal.target)?;
        match signal.signal {
            SignalKind::Interrupt => match entry.terminal.input(vec![INTERRUPT]) {
                Ok(_) | Err(Refused::Closed) => Ok(()),
                Err(Refused::Overflow { .. }) => Err(LinkError::InterruptRefused),
                Err(Refused::Exhausted) => Err(LinkError::CounterExhausted),
            },
            SignalKind::Terminate | SignalKind::Kill => match entry.terminal.kill() {
                Ok(()) | Err(ConptyError::ProcessExited) => Ok(()),
                Err(error) => Err(error.into()),
            },
        }
    }

    /// The retained output from the requested offset, flagged when it was evicted.
    pub(in crate::service::windows) fn tail(
        &self,
        request: &TailRequest,
    ) -> Result<Tail, LinkError> {
        let entry = self.entry(&request.target)?;
        let from = request.from_offset;
        let read = entry
            .terminal
            .read(OutputOffset::new(from), std::time::Duration::ZERO)?;
        let (first, bytes, gap) = match read {
            OutputRead::Bytes { from, bytes, gap } => (from.get(), bytes, gap),
            OutputRead::Pending | OutputRead::Ended(_) => (from, Vec::new(), false),
        };
        Ok(Tail {
            target: entry.target.clone(),
            from_offset: first,
            through_offset: first.saturating_add(count(bytes.len())),
            bytes,
            gap,
        })
    }

    /// Releases an exited terminal whose output was read through its final offset.
    pub(in crate::service::windows) fn forget(&self, forget: &Forget) -> Result<(), LinkError> {
        let entry = self.entry(&forget.target)?;
        let final_offset = lock(&entry.evidence)
            .exit
            .as_ref()
            .map(|exit| exit.final_offset);
        let acknowledged = forget.acknowledged_final_offset;
        let drained =
            matches!((final_offset, acknowledged), (Some(last), Some(read)) if read >= last);
        if !drained {
            return Err(LinkError::NotForgettable {
                terminal_id: forget.target.terminal_id.clone(),
            });
        }
        lock(&self.entries).remove(&forget.target.terminal_id);
        Ok(())
    }
}

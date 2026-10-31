//! Holder-owned terminal records and the ordered publication of their events.

use std::collections::BTreeMap;
use std::sync::Arc;

use portable_pty::MasterPty;

use super::super::error::HolderError;
use super::super::input::{InputQueue, QueuedInput};
use super::super::io_error::TerminalIoError;
use super::super::link::{LinkClosed, Outbox};
use super::super::tail::RetainedTail;
use super::ending::Completion;
use crate::process::ProcessId;
use crate::protocol::{
    AdoptionError, AdoptionTable, Hangup, HeldTerminal, InputWritten, Message, Output,
    ProcessIncarnation, PtyIoFailure, TerminalId, TerminalTarget,
};
use crate::unix::pty_io::PumpStop;

/// The running side of a terminal; dropped once its exit is recorded.
pub(super) struct LiveTerminal {
    pub(super) master: Box<dyn MasterPty + Send>,
    pub(super) input: Arc<InputQueue>,
    pub(super) pid: ProcessId,
    pub(super) pump_stop: Option<PumpStop>,
}

/// Process evidence is independent of the PTY/output completion barrier.
#[derive(Clone, Copy)]
pub(super) enum WaiterState {
    Waiting,
    Reaped,
}

pub(super) struct Terminal {
    pub(super) held: HeldTerminal,
    pub(super) tail: RetainedTail,
    pub(super) live: Option<LiveTerminal>,
    pub(super) geometry_epoch: Option<u64>,
    pub(super) admitted_sequence: Option<u64>,
    /// Unexpected failures with their sources; each worker records at most one.
    pub(super) io_errors: Vec<Arc<TerminalIoError>>,
    /// Wire form of the first failure before exit, carried by the exit record.
    pub(super) first_io_failure: Option<PtyIoFailure>,
    pub(super) completion: Completion,
    pub(super) waiter: WaiterState,
}

impl Terminal {
    pub(super) fn target(&self) -> TerminalTarget {
        TerminalTarget {
            terminal_id: self.held.terminal_id.clone(),
            terminal_incarnation: self.held.terminal_incarnation,
            process_incarnation: self.held.process_incarnation,
        }
    }
}

/// The write-boundary result for one input entry.
pub(super) struct WriteAck {
    pub(super) input: QueuedInput,
    pub(super) written: usize,
    pub(super) fence_epoch: u64,
    pub(super) output_offset: u64,
}

#[derive(Default)]
pub(super) struct HolderState {
    pub(super) terminals: BTreeMap<TerminalId, Terminal>,
    pub(super) draining: bool,
    link: Option<Arc<Outbox>>,
    last_process: u64,
    last_ack: u64,
}

impl HolderState {
    /// Publishes PTY EOF before the pump records Exit.
    ///
    /// The leader may already be reaped (a descendant can hold the slave long
    /// after it exits), so the evidence is the PID and start token retained at
    /// spawn, never a live lookup of a PID that may since have been reused.
    pub(super) fn record_hangup(&mut self, terminal_id: &TerminalId) {
        let Some(terminal) = self.terminals.get(terminal_id) else {
            return;
        };
        if terminal
            .io_errors
            .iter()
            .any(|error| error.exit_unknown().is_some())
        {
            return;
        }
        let (Some(pid), Some(start_token)) = (terminal.held.pid, &terminal.held.start_token) else {
            return;
        };
        let output = Output {
            target: terminal.target(),
            offset: terminal.tail.head(),
            bytes: Vec::new(),
            hangup: Some(Hangup {
                pid: ProcessId::new(pid),
                start_token: start_token.clone(),
            }),
        };
        self.publish(Message::Output(output));
    }

    /// Delivers to the adopted core, dropping a link that refuses delivery.
    pub(super) fn publish(&mut self, message: Message) {
        if self
            .link
            .as_ref()
            .is_some_and(|link| link.push(message).is_err())
        {
            self.link = None;
        }
    }

    pub(super) fn attach(&mut self, outbox: Arc<Outbox>) {
        if let Some(previous) = self.link.replace(outbox) {
            previous.close(LinkClosed::Replaced);
        }
    }

    pub(super) fn detach(&mut self, reason: LinkClosed) {
        if let Some(link) = self.link.take() {
            link.close(reason);
        }
    }

    pub(super) fn allocate_process(&mut self) -> Result<ProcessIncarnation, HolderError> {
        self.last_process = self
            .last_process
            .checked_add(1)
            .ok_or(HolderError::CounterExhausted)?;
        Ok(ProcessIncarnation::new(self.last_process))
    }

    /// Resolves a target whose terminal and (when given) process incarnation match.
    pub(super) fn terminal_mut(
        &mut self,
        target: &TerminalTarget,
    ) -> Result<&mut Terminal, HolderError> {
        let terminal = self.terminals.get_mut(&target.terminal_id).ok_or_else(|| {
            HolderError::UnknownTerminal {
                terminal_id: target.terminal_id.clone(),
            }
        })?;
        let process_matches = target
            .process_incarnation
            .is_none_or(|process| terminal.held.process_incarnation == Some(process));
        if terminal.held.terminal_incarnation != target.terminal_incarnation || !process_matches {
            return Err(HolderError::StaleTarget {
                terminal_id: target.terminal_id.clone(),
            });
        }
        Ok(terminal)
    }

    pub(super) fn adoption_table(&self) -> Result<AdoptionTable, AdoptionError> {
        let mut table = AdoptionTable::default();
        for terminal in self.terminals.values() {
            table.retain(terminal.held.clone())?;
        }
        Ok(table)
    }

    /// Appends pump output to the tail and publishes it in production order.
    pub(super) fn record_output(
        &mut self,
        terminal_id: &TerminalId,
        chunk: &[u8],
    ) -> Result<(), HolderError> {
        let Some(terminal) = self.terminals.get_mut(terminal_id) else {
            return Ok(());
        };
        let offset = terminal.tail.append(chunk)?;
        terminal.held.output_offset = terminal.tail.head();
        let target = terminal.target();
        self.publish(Message::Output(crate::protocol::Output {
            target,
            offset,
            bytes: chunk.to_vec(),
            hangup: None,
        }));
        Ok(())
    }

    /// Publishes the actual write-boundary byte count for one input entry.
    pub(super) fn acknowledge(&mut self, terminal_id: &TerminalId, ack: &WriteAck) {
        let Some(terminal) = self.terminals.get_mut(terminal_id) else {
            return;
        };
        terminal.held.input_sequence = terminal.held.input_sequence.max(ack.input.sequence);
        let target = terminal.target();
        // Saturating: a holder cannot issue 2^64 acknowledgements in practice, and a
        // repeated final epoch is preferable to wrapping backwards.
        self.last_ack = self.last_ack.saturating_add(1);
        self.publish(Message::InputWritten(InputWritten {
            target,
            sequence: ack.input.sequence,
            grant_epoch: ack.input.grant.grant_epoch,
            fence_epoch: ack.fence_epoch,
            ack_epoch: self.last_ack,
            written_bytes: byte_count(ack.written),
            output_offset: ack.output_offset,
        }));
    }
}

/// Converts an in-memory length to a wire count; lossless where `usize` has at most 64 bits.
pub(super) fn byte_count(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

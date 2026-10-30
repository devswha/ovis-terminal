//! How a terminal ends: exit evidence and unexpected I/O failures, kept typed.

use std::sync::Arc;

use super::state::HolderState;
use crate::protocol::{Exit, Message, TerminalId};
use crate::unix::io_error::TerminalIoError;

/// What the waiter observed about the process.
#[derive(Debug)]
pub(super) enum ExitEvidence {
    /// The OS reported a status; `None` fields mean it gave none.
    Status {
        code: Option<i32>,
        signal: Option<i32>,
    },
    /// No status could be observed.
    Unknown(TerminalIoError),
}

/// The waiter's report to the pump.
#[derive(Debug)]
pub(super) struct WaiterReport {
    pub(super) evidence: ExitEvidence,
    /// Failure while waiting for the master to drain before closing the slave.
    pub(super) drain: Option<TerminalIoError>,
}

/// Everything known when a terminal's output ended.
#[derive(Debug)]
pub(super) struct Ending {
    pub(super) evidence: ExitEvidence,
    /// Unexpected failure that ended output before a hang-up, if any.
    pub(super) output: Option<TerminalIoError>,
}

impl HolderState {
    /// Keeps a failure with its source; the first one before exit reaches the core.
    pub(super) fn record_io_error(&mut self, terminal_id: &TerminalId, error: TerminalIoError) {
        let Some(terminal) = self.terminals.get_mut(terminal_id) else {
            return;
        };
        if terminal.held.exit.is_none() && terminal.first_io_failure.is_none() {
            terminal.first_io_failure = error.io_failure();
        }
        terminal.io_errors.push(Arc::new(error));
    }

    /// Records exit after output ended, so `final_offset` covers every recorded byte.
    pub(super) fn record_exit(&mut self, terminal_id: &TerminalId, ending: Ending) {
        if let Some(error) = ending.output {
            self.record_io_error(terminal_id, error);
        }
        let Some(terminal) = self.terminals.get_mut(terminal_id) else {
            return;
        };
        let (code, signal, unknown) = match ending.evidence {
            ExitEvidence::Status { code, signal } => (code, signal, None),
            ExitEvidence::Unknown(error) => (None, None, Some(error)),
        };
        let exit = Exit {
            target: terminal.target(),
            code,
            signal,
            exit_unknown: unknown.as_ref().and_then(TerminalIoError::exit_unknown),
            io_failure: terminal.first_io_failure,
            final_offset: terminal.tail.head(),
        };
        if let Some(error) = unknown {
            terminal.io_errors.push(Arc::new(error));
        }
        terminal.held.exit = Some(exit.clone());
        if let Some(live) = terminal.live.take() {
            live.input.close();
        }
        self.publish(Message::Exit(exit));
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::super::state::{HolderState, Terminal};
    use super::{Ending, ExitEvidence};
    use crate::protocol::{
        ExitUnknown, ExitUnknownReason, HeldTerminal, PtyIoFailure, PtyOperation, TerminalId,
        TerminalIncarnation,
    };
    use crate::unix::io_error::TerminalIoError;
    use crate::unix::tail::RetainedTail;

    fn holding(terminal_id: &TerminalId) -> HolderState {
        let held = HeldTerminal {
            terminal_id: terminal_id.clone(),
            terminal_incarnation: TerminalIncarnation::new(1),
            process_incarnation: None,
            pid: None,
            start_token: None,
            output_offset: 0,
            input_sequence: 0,
            exit: None,
        };
        let terminal = Terminal {
            held,
            tail: RetainedTail::new(16),
            live: None,
            geometry_epoch: None,
            admitted_sequence: None,
            io_errors: Vec::new(),
            first_io_failure: None,
        };
        let mut state = HolderState::default();
        state.terminals.insert(terminal_id.clone(), terminal);
        state
    }

    #[test]
    fn failed_wait_and_read_are_typed_exit_evidence_not_a_null_exit() {
        // Given: a held terminal.
        let terminal_id = TerminalId::new("t-ending".to_owned());
        let mut state = holding(&terminal_id);

        // When: waiting failed with ECHILD and output ended with EISDIR.
        let ending = Ending {
            evidence: ExitEvidence::Unknown(TerminalIoError::Wait {
                source: io::Error::from_raw_os_error(libc::ECHILD),
            }),
            output: Some(TerminalIoError::Read {
                source: io::Error::from_raw_os_error(libc::EISDIR),
            }),
        };
        state.record_exit(&terminal_id, ending);

        // Then: the exit says why its status is unknown and how output ended,
        // and both failures keep their sources.
        let terminal = state.terminals.get(&terminal_id).expect("held");
        let exit = terminal.held.exit.clone().expect("exit recorded");
        assert_eq!((exit.code, exit.signal), (None, None));
        let unknown = ExitUnknown {
            reason: ExitUnknownReason::WaitFailed,
            os_error: Some(libc::ECHILD),
        };
        assert_eq!(exit.exit_unknown, Some(unknown));
        let failure = PtyIoFailure {
            operation: PtyOperation::Read,
            os_error: Some(libc::EISDIR),
            poll_revents: None,
        };
        assert_eq!(exit.io_failure, Some(failure));
        let kinds: Vec<&str> = terminal
            .io_errors
            .iter()
            .map(|error| error.kind())
            .collect();
        assert_eq!(kinds, ["pty_read_failed", "wait_failed"]);
        assert!(
            terminal
                .io_errors
                .iter()
                .all(|error| std::error::Error::source(error.as_ref()).is_some())
        );
    }
}

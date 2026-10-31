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

/// Output and input may finish in either order; the second side publishes Exit.
pub(super) enum Completion {
    Running,
    OutputEnded(Ending),
    InputEnded,
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

    /// Closes admission at EOF, retaining exit evidence until every input is acknowledged.
    pub(super) fn record_exit(&mut self, terminal_id: &TerminalId, mut ending: Ending) {
        if let Some(error) = ending.output.take() {
            self.record_io_error(terminal_id, error);
        }
        let Some(terminal) = self.terminals.get_mut(terminal_id) else {
            return;
        };
        if let Some(live) = terminal.live.take() {
            live.input.close();
        }
        match terminal.completion {
            Completion::Running | Completion::OutputEnded(_) => {
                terminal.completion = Completion::OutputEnded(ending);
            }
            Completion::InputEnded => self.publish_exit(terminal_id, ending),
        }
    }

    /// Called only after the writer drained the closed queue and published its last ack.
    pub(super) fn finish_input(&mut self, terminal_id: &TerminalId) {
        let Some(terminal) = self.terminals.get_mut(terminal_id) else {
            return;
        };
        match std::mem::replace(&mut terminal.completion, Completion::InputEnded) {
            Completion::OutputEnded(ending) => self.publish_exit(terminal_id, ending),
            Completion::Running | Completion::InputEnded => {}
        }
    }

    /// Both workers are finished; the FIFO socket writer preserves this publication order.
    fn publish_exit(&mut self, terminal_id: &TerminalId, ending: Ending) {
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
        self.publish(Message::Exit(exit));
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::sync::{Arc, mpsc};
    use std::{io, thread};

    use super::super::Holder;
    use super::super::state::{HolderState, Terminal};
    use super::super::threads::{InputSink, write_input};
    use super::{Completion, Ending, ExitEvidence};
    use crate::protocol::{
        ExitUnknown, ExitUnknownReason, HOLDER_PROTOCOL_GENERATION, HeldTerminal, Message,
        ProcessIncarnation, PtyIoFailure, PtyOperation, TerminalId, TerminalIncarnation,
        WriteGrant,
    };
    use crate::unix::config::QueueLimits;
    use crate::unix::input::{InputQueue, QueuedInput};
    use crate::unix::io_error::TerminalIoError;
    use crate::unix::tail::RetainedTail;
    use crate::unix::test_fixtures::{WAIT, config, hello_record};

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
            completion: Completion::Running,
            waiter: super::super::state::WaiterState::Waiting,
        };
        let mut state = HolderState::default();
        state.terminals.insert(terminal_id.clone(), terminal);
        state
    }

    #[test]
    fn exit_waits_for_gated_writer_to_acknowledge_closed_queue() {
        // Given: two admitted entries whose real input writer is held at a gate.
        let terminal_id = TerminalId::new("t-exit-order".to_owned());
        let holder = Holder::new(config());
        *holder.shared.lock() = holding(&terminal_id);
        let link = holder
            .adopt(&hello_record(HOLDER_PROTOCOL_GENERATION))
            .expect("adopt");
        assert!(matches!(link.recv_timeout(WAIT), Ok(Message::Welcome(_))));
        let queue = Arc::new(InputQueue::new(QueueLimits::new(16, 2)));
        for sequence in 1..=2 {
            let admission = queue.push(QueuedInput {
                sequence,
                grant: WriteGrant {
                    grant_epoch: 1,
                    fence_epoch: 1,
                    process_incarnation: ProcessIncarnation::new(1),
                },
                bytes: b"go\n".to_vec(),
            });
            assert!(admission.outcome.is_ok());
        }
        let sink = InputSink {
            terminal_id: terminal_id.clone(),
            queue: Arc::clone(&queue),
            file: File::options().write(true).open("/dev/null").expect("sink"),
        };
        let (release, gate) = mpsc::sync_channel(1);
        let (done, finished) = mpsc::sync_channel(1);
        let shared = Arc::clone(&holder.shared);
        let writer = thread::spawn(move || {
            gate.recv_timeout(WAIT).expect("release writer");
            write_input(&shared, sink);
            done.send(()).expect("writer completed");
        });

        // When: output ends and exit is recorded before the writer can acknowledge.
        queue.close();
        {
            let mut state = holder.shared.lock();
            state.record_output(&terminal_id, b"final").expect("output");
            state.record_exit(
                &terminal_id,
                Ending {
                    evidence: ExitEvidence::Status {
                        code: Some(7),
                        signal: None,
                    },
                    output: None,
                },
            );
        }
        release.send(()).expect("release gate");
        finished
            .recv_timeout(WAIT)
            .expect("bounded writer completion");
        writer.join().expect("writer joined");

        // Then: output and both actual unwritten acknowledgements precede Exit.
        let mut exited = false;
        let mut sequences = Vec::new();
        for _ in 0..4 {
            let message = link.recv_timeout(WAIT).expect("terminal event");
            assert!(!exited, "{:?} followed Exit", message.kind());
            match message {
                Message::Output(output) => assert_eq!(output.bytes, b"final"),
                Message::InputWritten(ack) => {
                    assert_eq!(ack.written_bytes, 0);
                    sequences.push(ack.sequence);
                }
                Message::Exit(exit) => {
                    assert_eq!((exit.code, exit.final_offset), (Some(7), 5));
                    exited = true;
                }
                other => panic!("unexpected {:?}", other.kind()),
            }
        }
        assert_eq!(sequences, [1, 2]);
        assert!(exited);
    }

    #[test]
    fn failed_wait_and_read_are_typed_exit_evidence_not_a_null_exit() {
        // Given: a held terminal.
        let terminal_id = TerminalId::new("t-ending".to_owned());
        let mut state = holding(&terminal_id);
        state.finish_input(&terminal_id);

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

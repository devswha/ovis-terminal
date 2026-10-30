//! Bodies of a terminal's exit waiter, output pump and input writer.
//!
//! Output order is fixed by construction: the pump records every chunk, reaches
//! the PTY hang-up, and only then records the exit the waiter observed, so
//! `final_offset` always covers the final bytes. Unexpected failures are kept
//! typed with their sources and reach the core in the exit record.

use std::fs::File;
use std::os::fd::OwnedFd;
use std::os::unix::process::ExitStatusExt;
use std::process::Child;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender};

use portable_pty::SlavePty;

use super::Shared;
use super::ending::{Ending, ExitEvidence, WaiterReport};
use super::state::WriteAck;
use crate::protocol::TerminalId;
use crate::unix::drain::DrainSignal;
use crate::unix::input::{Dequeued, InputQueue};
use crate::unix::io_error::TerminalIoError;
use crate::unix::pty_io::{ReadOutcome, read_chunk, write_prefix};

/// Output is read in chunks no larger than the stream chunk size.
const OUTPUT_CHUNK: usize = 16 * 1024;

/// Marks the pump finished when dropped, including when its thread never started.
pub(super) struct FinishOnDrop(pub(super) Arc<DrainSignal>);

impl Drop for FinishOnDrop {
    fn drop(&mut self) {
        self.0.finish();
    }
}

pub(super) struct Waiter {
    pub(super) shared: Arc<Shared>,
    pub(super) terminal_id: TerminalId,
    pub(super) child: Child,
    pub(super) slave: Box<dyn SlavePty + Send>,
    pub(super) probe: OwnedFd,
    pub(super) drain: Arc<DrainSignal>,
}

pub(super) fn wait_for_exit(waiter: Waiter, report: &SyncSender<WaiterReport>) {
    let Waiter {
        shared,
        terminal_id,
        mut child,
        slave,
        probe,
        drain,
    } = waiter;
    let evidence = match child.wait() {
        Ok(status) => ExitEvidence::Status {
            code: status.code(),
            signal: status.signal(),
        },
        Err(source) => ExitEvidence::Unknown(TerminalIoError::Wait { source }),
    };
    // A failed pending-byte query must not hold the exit forever: the slave is
    // closed anyway, the pump records what the kernel still returns, and the
    // failure is reported with the exit.
    let drained = drain.wait_until_drained(&probe);
    if drained.is_err() {
        drain.finish();
    }
    drop(slave);
    let report_value = WaiterReport {
        evidence,
        drain: drained.err().map(|source| TerminalIoError::Poll { source }),
    };
    // The pump only stops listening if its thread is gone; record the exit here then.
    if let Err(unsent) = report.send(report_value) {
        let ending = Ending {
            evidence: unsent.0.evidence,
            output: Some(TerminalIoError::PumpLost),
        };
        shared.lock().record_exit(&terminal_id, ending);
    }
}

pub(super) struct Pump {
    pub(super) terminal_id: TerminalId,
    pub(super) reader: File,
    pub(super) finish: FinishOnDrop,
    pub(super) report: Receiver<WaiterReport>,
}

pub(super) fn pump_output(shared: &Shared, pump: Pump) {
    let Pump {
        terminal_id,
        mut reader,
        finish,
        report,
    } = pump;
    let mut buffer = vec![0_u8; OUTPUT_CHUNK];
    loop {
        let failure = match read_chunk(&mut reader, &mut buffer) {
            Ok(ReadOutcome::Hangup) => break,
            Ok(ReadOutcome::Data(read)) => buffer
                .get(..read)
                .and_then(|chunk| shared.lock().record_output(&terminal_id, chunk).err())
                .map(|source| TerminalIoError::Record { source }),
            Err(failure) => Some(failure),
        };
        if let Some(failure) = failure {
            // Recorded now, before the exit, so the exit record carries it; the
            // program keeps running until it exits or the core stops it.
            shared.lock().record_io_error(&terminal_id, failure);
            break;
        }
        finish.0.advance();
    }
    drop(finish);
    let ending = match report.recv() {
        Ok(WaiterReport { evidence, drain }) => Ending {
            evidence,
            output: drain,
        },
        Err(_) => Ending {
            evidence: ExitEvidence::Unknown(TerminalIoError::WaiterLost),
            output: None,
        },
    };
    shared.lock().record_exit(&terminal_id, ending);
}

pub(super) struct InputSink {
    pub(super) terminal_id: TerminalId,
    pub(super) queue: Arc<InputQueue>,
    pub(super) file: File,
}

pub(super) fn write_input(shared: &Shared, sink: InputSink) {
    let InputSink {
        terminal_id,
        queue,
        mut file,
    } = sink;
    let mut writable = true;
    while let Some(Dequeued { input, fence_epoch }) = queue.pop() {
        let output_offset = shared
            .lock()
            .terminals
            .get(&terminal_id)
            .map_or(0, |terminal| terminal.tail.head());
        let fence_epoch = queue.fence_epoch().max(fence_epoch);
        // Revoked entries, anything queued after the process exited, and anything
        // after a write failure are acknowledged without touching the PTY.
        let written = if !writable || input.grant.fence_epoch < fence_epoch || queue.is_closed() {
            0
        } else {
            let outcome = write_prefix(&mut file, &input.bytes);
            if let Some(failure) = outcome.failure {
                writable = false;
                shared.lock().record_io_error(&terminal_id, failure);
            }
            outcome.written
        };
        shared.lock().acknowledge(
            &terminal_id,
            &WriteAck {
                input,
                written,
                fence_epoch,
                output_offset,
            },
        );
    }
}

//! The three threads each terminal owns: exit waiter, output pump and input writer.
//!
//! Output order is fixed by construction: the pump records every chunk, reaches
//! PTY EOF, and only then records the exit the waiter observed, so
//! `final_offset` always covers the final bytes.

use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::process::ExitStatusExt;
use std::process::Child;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, PoisonError};
use std::thread::{Builder, JoinHandle};

use portable_pty::{MasterPty, SlavePty};

use super::Shared;
use super::state::{ExitEvidence, HolderState, WriteAck};
use crate::process::{ProcessId, SignalError, SignalScope, send_signal};
use crate::protocol::{SignalKind, TerminalId};
use crate::unix::drain::DrainSignal;
use crate::unix::input::{Dequeued, InputQueue};
use crate::unix::launch::PtyProcess;
use crate::unix::pty_io::{ReadOutcome, read_chunk, write_prefix};

/// Output is read in chunks no larger than the stream chunk size.
const OUTPUT_CHUNK: usize = 16 * 1024;

/// A launched process split into the master kept by the holder and thread-owned parts.
pub(super) struct Parts {
    pub(super) master: Box<dyn MasterPty + Send>,
    pub(super) threads: ThreadParts,
}

pub(super) struct ThreadParts {
    child: Child,
    slave: Box<dyn SlavePty + Send>,
    probe: OwnedFd,
    reader: File,
    writer: File,
    input: Arc<InputQueue>,
}

impl Parts {
    pub(super) fn split(process: PtyProcess, input: Arc<InputQueue>) -> Self {
        Self {
            master: process.master,
            threads: ThreadParts {
                child: process.child,
                slave: process.slave,
                probe: process.probe,
                reader: process.reader,
                writer: process.writer,
                input,
            },
        }
    }
}

/// Marks the pump finished when dropped, including when its thread never started.
struct FinishOnDrop(Arc<DrainSignal>);

impl Drop for FinishOnDrop {
    fn drop(&mut self) {
        self.0.finish();
    }
}

/// Starts the waiter, pump and writer; each handle is registered for joining.
pub(super) fn start(
    shared: &Arc<Shared>,
    terminal_id: &TerminalId,
    parts: ThreadParts,
) -> io::Result<()> {
    let drain = Arc::new(DrainSignal::default());
    let (status_tx, status_rx) = mpsc::sync_channel(1);
    let ThreadParts {
        child,
        slave,
        probe,
        reader,
        writer,
        input,
    } = parts;
    let waiter_drain = Arc::clone(&drain);
    let waiter = Waiter {
        child,
        slave,
        probe,
        drain: waiter_drain,
    };
    let waiter = move || wait_for_exit(waiter, &status_tx);
    register(shared, spawn_named("ovis-holder-wait", waiter)?);
    let pump_shared = Arc::clone(shared);
    let pump = Pump {
        terminal_id: terminal_id.clone(),
        reader,
        finish: FinishOnDrop(drain),
        status: status_rx,
    };
    register(
        shared,
        spawn_named("ovis-holder-pump", move || pump_output(&pump_shared, pump))?,
    );
    let writer_shared = Arc::clone(shared);
    let sink = InputSink {
        terminal_id: terminal_id.clone(),
        queue: input,
        file: writer,
    };
    let writer = move || write_input(&writer_shared, sink);
    register(shared, spawn_named("ovis-holder-input", writer)?);
    Ok(())
}

/// Removes a terminal whose workers could not start and kills its process group.
pub(super) fn abandon(
    state: &mut HolderState,
    terminal_id: &TerminalId,
    pid: ProcessId,
) -> Result<(), SignalError> {
    if let Some(live) = state
        .terminals
        .remove(terminal_id)
        .and_then(|terminal| terminal.live)
    {
        live.input.close();
    }
    send_signal(SignalScope::ProcessGroup(pid), SignalKind::Kill)
}

fn spawn_named<F>(name: &str, body: F) -> io::Result<JoinHandle<()>>
where
    F: FnOnce() + Send + 'static,
{
    Builder::new().name(name.to_owned()).spawn(body)
}

/// Keeps `handle` for shutdown, joining any workers that already finished.
fn register(shared: &Shared, handle: JoinHandle<()>) {
    let mut workers = shared
        .workers
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let (finished, running): (Vec<_>, Vec<_>) = std::mem::take(&mut workers.running)
        .into_iter()
        .partition(JoinHandle::is_finished);
    workers.running = running;
    for done in finished {
        if done.join().is_err() {
            workers.panicked = workers.panicked.saturating_add(1);
        }
    }
    workers.running.push(handle);
}

struct Waiter {
    child: Child,
    slave: Box<dyn SlavePty + Send>,
    probe: OwnedFd,
    drain: Arc<DrainSignal>,
}

fn wait_for_exit(waiter: Waiter, status: &SyncSender<ExitEvidence>) {
    let Waiter {
        mut child,
        slave,
        probe,
        drain,
    } = waiter;
    let evidence = child.wait().map_or(
        ExitEvidence {
            code: None,
            signal: None,
        },
        |exit| ExitEvidence {
            code: exit.code(),
            signal: exit.signal(),
        },
    );
    // An unreadable pending count must not hold the exit forever: the slave is
    // closed anyway and the pump records whatever the kernel still returns.
    if drain.wait_until_drained(&probe).is_err() {
        drain.finish();
    }
    drop(slave);
    // A pump that already ended has nothing left to record; the evidence goes unused.
    status.send(evidence).unwrap_or_default();
}

struct Pump {
    terminal_id: TerminalId,
    reader: File,
    finish: FinishOnDrop,
    status: Receiver<ExitEvidence>,
}

fn pump_output(shared: &Shared, pump: Pump) {
    let Pump {
        terminal_id,
        mut reader,
        finish,
        status,
    } = pump;
    let mut buffer = vec![0_u8; OUTPUT_CHUNK];
    while let ReadOutcome::Data(read) = read_chunk(&mut reader, &mut buffer) {
        let recorded = buffer
            .get(..read)
            .map(|chunk| shared.lock().record_output(&terminal_id, chunk));
        if !matches!(recorded, Some(Ok(()))) {
            break;
        }
        finish.0.advance();
    }
    drop(finish);
    let evidence = status.recv().unwrap_or(ExitEvidence {
        code: None,
        signal: None,
    });
    shared.lock().record_exit(&terminal_id, evidence);
}

struct InputSink {
    terminal_id: TerminalId,
    queue: Arc<InputQueue>,
    file: File,
}

fn write_input(shared: &Shared, sink: InputSink) {
    let InputSink {
        terminal_id,
        queue,
        mut file,
    } = sink;
    while let Some(Dequeued { input, fence_epoch }) = queue.pop() {
        let output_offset = shared
            .lock()
            .terminals
            .get(&terminal_id)
            .map_or(0, |terminal| terminal.tail.head());
        let fence_epoch = queue.fence_epoch().max(fence_epoch);
        // Revoked entries, and anything still queued after the process exited, are
        // acknowledged without touching the PTY.
        let written = if input.grant.fence_epoch < fence_epoch || queue.is_closed() {
            0
        } else {
            write_prefix(&mut file, &input.bytes)
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

//! Starting and joining the three threads each terminal owns.

use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;
use std::process::Child;
use std::sync::{Arc, PoisonError, mpsc};
use std::thread::{Builder, JoinHandle};

use portable_pty::{MasterPty, SlavePty};

use super::Shared;
use super::state::HolderState;
use super::threads::{
    FinishOnDrop, InputSink, Pump, Waiter, pump_output, wait_for_exit, write_input,
};
use crate::process::{ProcessId, SignalError, SignalScope, send_signal};
use crate::protocol::{SignalKind, TerminalId};
use crate::unix::drain::DrainSignal;
use crate::unix::input::InputQueue;
use crate::unix::launch::PtyProcess;

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

/// Starts the waiter, pump and writer; each handle is registered for joining.
pub(super) fn start(
    shared: &Arc<Shared>,
    terminal_id: &TerminalId,
    parts: ThreadParts,
) -> io::Result<()> {
    let drain = Arc::new(DrainSignal::default());
    let (report_tx, report_rx) = mpsc::sync_channel(1);
    let ThreadParts {
        child,
        slave,
        probe,
        reader,
        writer,
        input,
    } = parts;
    let waiter = Waiter {
        shared: Arc::clone(shared),
        terminal_id: terminal_id.clone(),
        child,
        slave,
        probe,
        drain: Arc::clone(&drain),
    };
    let waiter = move || wait_for_exit(waiter, &report_tx);
    register(shared, spawn_named("ovis-holder-wait", waiter)?);
    let pump_shared = Arc::clone(shared);
    let pump = Pump {
        terminal_id: terminal_id.clone(),
        reader,
        finish: FinishOnDrop(drain),
        report: report_rx,
    };
    let pump = move || pump_output(&pump_shared, pump);
    register(shared, spawn_named("ovis-holder-pump", pump)?);
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

//! The three threads of a pseudoconsole terminal and the state they share.
//!
//! The reader always drains output into the bounded log, the writer empties the bounded
//! input queue, and the waiter observes the program end. Whichever thread observes the
//! last teardown fact publishes the exit; the console is closed by the thread that sees
//! the program exit, while the reader is still draining.

use std::io::{self, Read, Write};
use std::num::NonZeroUsize;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use portable_pty::{Child, MasterPty};

use super::model::{
    InputQueue, InputSequence, OutputEnd, OutputLog, ProgramExit, Teardown, TeardownEvent,
};

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Last input written whole to the pseudoconsole, and the write failure that ended input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct InputProgress {
    /// Highest sequence whose bytes were all written.
    pub written: Option<InputSequence>,
    /// Set when a write failed; the OS error number, when one was reported.
    pub failed: Option<Option<i32>>,
}

pub(super) struct Shared {
    pub(super) output: Mutex<OutputLog>,
    pub(super) output_changed: Condvar,
    pub(super) input: Mutex<InputQueue>,
    input_ready: Condvar,
    pub(super) progress: Mutex<InputProgress>,
    pub(super) console: Mutex<Option<Box<dyn MasterPty + Send>>>,
    teardown: Mutex<Teardown>,
}

impl Shared {
    pub(super) fn new(
        output: OutputLog,
        input: InputQueue,
        console: Box<dyn MasterPty + Send>,
    ) -> Self {
        Self {
            output: Mutex::new(output),
            output_changed: Condvar::new(),
            input: Mutex::new(input),
            input_ready: Condvar::new(),
            progress: Mutex::new(InputProgress::default()),
            console: Mutex::new(Some(console)),
            teardown: Mutex::new(Teardown::default()),
        }
    }

    pub(super) fn notify_input(&self) {
        self.input_ready.notify_all();
    }

    pub(super) fn close_input(&self) {
        lock(&self.input).close();
        self.input_ready.notify_all();
    }

    /// Closes the pseudoconsole; ConPTY then ends its output pipe after the last byte.
    pub(super) fn close_console(&self) {
        let console = lock(&self.console).take();
        drop(console);
    }

    fn observe(&self, event: TeardownEvent) {
        let steps = lock(&self.teardown).observe(event);
        if steps.close_console {
            self.close_input();
            self.close_console();
        }
        if let Some((exit, output)) = steps.finished {
            lock(&self.output).end(exit, output);
            self.output_changed.notify_all();
        }
    }

    fn next_input(&self) -> Option<(InputSequence, Vec<u8>)> {
        let mut queue = lock(&self.input);
        loop {
            if let Some(entry) = queue.pop() {
                return Some(entry);
            }
            if queue.is_closed() {
                return None;
            }
            queue = self
                .input_ready
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

fn read_output(
    shared: &Shared,
    mut reader: Box<dyn Read + Send>,
    chunk: NonZeroUsize,
) -> OutputEnd {
    let mut buffer = vec![0; chunk.get()];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return OutputEnd::Eof,
            Ok(length) => {
                let bytes = buffer.get(..length).unwrap_or_default();
                if lock(&shared.output).append(bytes).is_err() {
                    return OutputEnd::Failed { os_error: None };
                }
                shared.output_changed.notify_all();
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => return OutputEnd::Eof,
            Err(error) => {
                return OutputEnd::Failed {
                    os_error: error.raw_os_error(),
                };
            }
        }
    }
}

fn write_input(shared: &Shared, mut writer: Box<dyn Write + Send>) {
    while let Some((sequence, bytes)) = shared.next_input() {
        if let Err(error) = writer.write_all(&bytes).and_then(|()| writer.flush()) {
            lock(&shared.progress).failed = Some(error.raw_os_error());
            shared.close_input();
            return;
        }
        lock(&shared.progress).written = Some(sequence);
    }
}

fn wait_program(mut child: Box<dyn Child + Send + Sync>) -> ProgramExit {
    match child.wait() {
        Ok(status) => ProgramExit::Code(status.exit_code()),
        Err(error) => ProgramExit::WaitFailed {
            os_error: error.raw_os_error(),
        },
    }
}

type Work = Box<dyn FnOnce() + Send>;

fn start(name: &str, work: Work) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name(format!("conpty-{name}"))
        .spawn(work)
}

pub(super) struct Ends {
    pub(super) reader: Box<dyn Read + Send>,
    pub(super) writer: Box<dyn Write + Send>,
    pub(super) child: Box<dyn Child + Send + Sync>,
    pub(super) chunk: NonZeroUsize,
}

/// Starts the waiter first, so an exit can always close the console; stops at the first
/// thread that cannot start and returns the ones that did.
pub(super) fn spawn(shared: &Arc<Shared>, ends: Ends) -> (Vec<JoinHandle<()>>, Option<io::Error>) {
    let Ends {
        reader,
        writer,
        child,
        chunk,
    } = ends;
    let mut started = Vec::with_capacity(3);
    let waiter = Arc::clone(shared);
    let work: [(&str, Work); 3] = [
        (
            "waiter",
            Box::new(move || waiter.observe(TeardownEvent::ProgramExited(wait_program(child)))),
        ),
        ("reader", {
            let shared = Arc::clone(shared);
            Box::new(move || {
                let end = read_output(&shared, reader, chunk);
                shared.observe(TeardownEvent::OutputEnded(end));
            })
        }),
        ("writer", {
            let shared = Arc::clone(shared);
            Box::new(move || write_input(&shared, writer))
        }),
    ];
    for (name, work) in work {
        match start(name, work) {
            Ok(handle) => started.push(handle),
            Err(error) => return (started, Some(error)),
        }
    }
    (started, None)
}

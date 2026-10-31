//! One program on one pseudoconsole, owned from spawn to its published exit.

use std::ffi::OsString;
use std::fmt;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use ovis_terminal_protocol::domain::conpty::{ConptyGrid, ConptyLaunch};
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

use super::error::ConptyError;
use super::job::Job;
use super::model::{
    InputLimits, InputQueue, InputRejected, InputSequence, OutputLog, OutputOffset, OutputRead,
};
use super::process::ProcessHandle;
use super::workers::{self, Ends, InputProgress, Shared, lock};
use crate::protocol::ProcessStartToken;

/// Exit code of a program ended by [`ConptyTerminal::kill`]: 128 plus `SIGKILL`'s
/// number, so a kill is not mistaken for an ordinary failure exit.
pub const KILL_EXIT_CODE: u32 = 137;

/// Per-terminal memory bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TerminalLimits {
    /// Output bytes retained for readers.
    pub output: NonZeroUsize,
    /// Input that may wait for the pseudoconsole.
    pub input: InputLimits,
    /// Largest single read of output, and of one returned output range.
    pub chunk: NonZeroUsize,
}

impl TerminalLimits {
    /// One MiB of retained output, the default input bound and 64 KiB reads.
    pub const DEFAULT: Self = Self {
        output: kib(1024),
        input: InputLimits::DEFAULT,
        chunk: kib(64),
    };
}

impl Default for TerminalLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

const fn kib(count: usize) -> NonZeroUsize {
    match NonZeroUsize::new(count.saturating_mul(1024)) {
        Some(size) => size,
        None => NonZeroUsize::MIN,
    }
}

/// A program running on a pseudoconsole, its job, and the threads that drain it.
///
/// Dropping the terminal stops admission, closes the job (ending the whole process
/// tree), closes the pseudoconsole and joins every worker; call [`Self::close`] to
/// learn whether a worker panicked.
pub struct ConptyTerminal {
    shared: Arc<Shared>,
    job: Option<Job>,
    /// Held so the PID cannot be reused while this terminal names it.
    _process: ProcessHandle,
    pid: u32,
    start_token: ProcessStartToken,
    limits: TerminalLimits,
    workers: Vec<JoinHandle<()>>,
}

impl fmt::Debug for ConptyTerminal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConptyTerminal")
            .field("pid", &self.pid)
            .field("start_token", &self.start_token)
            .finish_non_exhaustive()
    }
}

fn command(launch: &ConptyLaunch) -> CommandBuilder {
    let mut command = CommandBuilder::from_argv(launch.argv().iter().map(OsString::from).collect());
    command.env_clear();
    for (name, value) in launch.env() {
        command.env(name, value);
    }
    command.cwd(launch.cwd());
    command
}

fn own(child: &dyn Child) -> std::io::Result<(Job, ProcessHandle, u32, ProcessStartToken)> {
    let process = ProcessHandle::of(child)?;
    let job = Job::new()?;
    job.assign(process.borrow())?;
    let pid = process.pid()?;
    let start_token = process.start_token()?;
    Ok((job, process, pid, start_token))
}

impl ConptyTerminal {
    /// Starts `launch` on a new pseudoconsole and takes ownership of its process tree.
    ///
    /// # Errors
    ///
    /// `spawn_failed` kinds ([`ConptyError::CwdUnavailable`],
    /// [`ConptyError::PseudoconsoleUnavailable`], [`ConptyError::ProgramUnavailable`],
    /// [`ConptyError::Ownership`]) when no program is left running, and
    /// [`ConptyError::WorkerUnavailable`] after stopping a started program.
    pub fn spawn(launch: &ConptyLaunch, limits: TerminalLimits) -> Result<Self, ConptyError> {
        let cwd = launch.cwd();
        let unavailable = |source| ConptyError::CwdUnavailable {
            cwd: cwd.clone(),
            source,
        };
        if !std::fs::metadata(cwd).map_err(unavailable)?.is_dir() {
            return Err(unavailable(std::io::ErrorKind::NotADirectory.into()));
        }
        let size = PtySize {
            rows: launch.grid().rows(),
            cols: launch.grid().cols(),
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system()
            .openpty(size)
            .map_err(ConptyError::pseudoconsole)?;
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(ConptyError::pseudoconsole)?;
        let writer = pair
            .master
            .take_writer()
            .map_err(ConptyError::pseudoconsole)?;
        let mut child = pair
            .slave
            .spawn_command(command(launch))
            .map_err(ConptyError::program)?;
        drop(pair.slave);
        let (job, process, pid, start_token) = own(child.as_ref()).map_err(|source| {
            let cleanup = child.kill().err();
            ConptyError::Ownership { source, cleanup }
        })?;
        let shared = Arc::new(Shared::new(
            OutputLog::new(limits.output),
            InputQueue::new(limits.input),
            pair.master,
        ));
        let ends = Ends {
            reader,
            writer,
            child,
            chunk: limits.chunk,
        };
        let (started, failure) = workers::spawn(&shared, ends);
        let terminal = Self {
            shared,
            job: Some(job),
            _process: process,
            pid,
            start_token,
            limits,
            workers: started,
        };
        failure.map_or(Ok(terminal), |source| {
            Err(ConptyError::WorkerUnavailable { source })
        })
    }

    /// Process identifier, reserved while this terminal holds the process handle.
    #[must_use]
    pub const fn pid(&self) -> u32 {
        self.pid
    }

    /// `windows-created:<FILETIME>` creation time read through the held handle.
    #[must_use]
    pub const fn start_token(&self) -> &ProcessStartToken {
        &self.start_token
    }

    /// Queues `bytes` for the pseudoconsole, whole or not at all.
    ///
    /// # Errors
    ///
    /// [`InputRejected`] before any byte is accepted.
    pub fn input(&self, bytes: Vec<u8>) -> Result<InputSequence, InputRejected> {
        let sequence = lock(&self.shared.input).push(bytes)?;
        self.shared.notify_input();
        Ok(sequence)
    }

    /// What has been written so far, and whether writing failed.
    #[must_use]
    pub fn input_progress(&self) -> InputProgress {
        *lock(&self.shared.progress)
    }

    /// Resizes the pseudoconsole without waiting behind input.
    ///
    /// # Errors
    ///
    /// [`ConptyError::ProcessExited`] once the console is closed, and
    /// [`ConptyError::Resize`] when ConPTY rejects the grid.
    pub fn resize(&self, grid: ConptyGrid) -> Result<(), ConptyError> {
        let size = PtySize {
            rows: grid.rows(),
            cols: grid.cols(),
            pixel_width: 0,
            pixel_height: 0,
        };
        lock(&self.shared.console)
            .as_ref()
            .ok_or(ConptyError::ProcessExited)?
            .resize(size)
            .map_err(ConptyError::resize)
    }

    /// Terminates the program's whole process tree with [`KILL_EXIT_CODE`].
    ///
    /// This requests termination; the exit read from [`Self::read`] is the evidence that
    /// the program ended.
    ///
    /// # Errors
    ///
    /// [`ConptyError::Kill`] when the job could not be terminated.
    pub fn kill(&self) -> Result<(), ConptyError> {
        let job = self.job.as_ref().ok_or(ConptyError::ProcessExited)?;
        job.terminate(KILL_EXIT_CODE)
            .map_err(|source| ConptyError::Kill { source })
    }

    /// Reads output from `from`, waiting at most `wait` while nothing new is there.
    ///
    /// Returns [`OutputRead::Ended`] only to a reader at the final offset.
    ///
    /// # Errors
    ///
    /// [`ConptyError::Output`] when `from` is past the newest byte.
    pub fn read(&self, from: OutputOffset, wait: Duration) -> Result<OutputRead, ConptyError> {
        let (log, _) = self
            .shared
            .output_changed
            .wait_timeout_while(lock(&self.shared.output), wait, |log| {
                log.head() == from && log.ended().is_none()
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let read = log.read(from, self.limits.chunk);
        drop(log);
        Ok(read?)
    }

    #[cfg(test)]
    pub(super) fn active_processes(&self) -> std::io::Result<u32> {
        self.job.as_ref().map_or(Ok(0), Job::active_processes)
    }

    fn release(&mut self) -> usize {
        self.shared.close_input();
        drop(self.job.take());
        self.shared.close_console();
        self.workers
            .drain(..)
            .filter_map(|worker| worker.join().err())
            .count()
    }

    /// Ends the program if it still runs, joins every worker and reports panics.
    ///
    /// # Errors
    ///
    /// [`ConptyError::WorkerPanicked`] when a worker panicked.
    pub fn close(mut self) -> Result<(), ConptyError> {
        match self.release() {
            0 => Ok(()),
            count => Err(ConptyError::WorkerPanicked { count }),
        }
    }
}

impl Drop for ConptyTerminal {
    fn drop(&mut self) {
        self.release();
    }
}

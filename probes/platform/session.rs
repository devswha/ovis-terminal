//! A real pseudo-terminal session driven through the pinned portable-pty backend.

use std::io::{self, Write};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::Duration;
use std::{fmt, mem};

use portable_pty::{
    ChildKiller, CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system,
};
use serde::Serialize;

use super::error::{ProbeError, PtyOperation};
use super::measure::ByteCount;
use super::output::{OutputStream, StreamEnd};
use super::process::Pid;
use super::pump::{spawn_reader, spawn_waiter};

/// Terminal size in character cells.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Grid {
    /// Columns.
    pub columns: u16,
    /// Rows.
    pub rows: u16,
}

impl Grid {
    /// A grid of `columns` x `rows` cells.
    #[must_use]
    pub const fn new(columns: u16, rows: u16) -> Self {
        Self { columns, rows }
    }

    const fn pty_size(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.columns,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

/// The exit status the backend collected.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ExitRecord {
    /// Exit code (1 when the program was ended by a signal).
    pub code: u32,
    /// Terminating signal name, when a signal ended the program.
    pub signal: Option<String>,
}

impl From<&ExitStatus> for ExitRecord {
    fn from(status: &ExitStatus) -> Self {
        Self {
            code: status.exit_code(),
            signal: status.signal().map(str::to_owned),
        }
    }
}

/// One program running in a real pseudo-terminal.
pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Option<Box<dyn Write + Send>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    output: OutputStream,
    exit: Receiver<io::Result<ExitStatus>>,
    exited: Option<ExitRecord>,
    pid: Option<Pid>,
    threads: Vec<JoinHandle<()>>,
}

impl fmt::Debug for PtySession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PtySession")
            .field("pid", &self.pid)
            .field("output", &self.output)
            .field("exited", &self.exited)
            .finish_non_exhaustive()
    }
}

fn backend(
    operation: PtyOperation,
) -> impl Fn(Box<dyn std::error::Error + Send + Sync>) -> ProbeError {
    move |source| ProbeError::Backend { operation, source }
}

impl PtySession {
    /// Opens a terminal of `grid` and starts `command` in it.
    ///
    /// # Errors
    ///
    /// [`ProbeError::Unavailable`] when no terminal pair can be created,
    /// [`ProbeError::SpawnFailed`] when the program cannot start, and
    /// [`ProbeError::Backend`] / [`ProbeError::Io`] when the session's reader,
    /// writer or threads cannot be set up.
    pub fn spawn(command: CommandBuilder, grid: Grid) -> Result<Self, ProbeError> {
        let pair = native_pty_system()
            .openpty(grid.pty_size())
            .map_err(|error| ProbeError::Unavailable {
                source: error.into(),
            })?;
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| ProbeError::SpawnFailed {
                source: error.into(),
            })?;
        drop(pair.slave);
        let master = pair.master;
        let reader = master
            .try_clone_reader()
            .map_err(|error| backend(PtyOperation::CloneReader)(error.into()))?;
        let writer = master
            .take_writer()
            .map_err(|error| backend(PtyOperation::TakeWriter)(error.into()))?;
        let pid = child.process_id().map(Pid::new);
        let killer = child.clone_killer();
        let (events, reader_thread) = spawn_reader(reader)?;
        let (exit, waiter_thread) = spawn_waiter(child)?;
        Ok(Self {
            master,
            writer: Some(writer),
            killer,
            output: OutputStream::new(events),
            exit,
            exited: None,
            pid,
            threads: vec![reader_thread, waiter_thread],
        })
    }

    /// The program's PID, when the backend reports one.
    #[must_use]
    pub const fn pid(&self) -> Option<Pid> {
        self.pid
    }

    /// Total output bytes received so far.
    #[must_use]
    pub const fn received(&self) -> ByteCount {
        self.output.received()
    }

    /// Output received but not yet consumed by a read, for diagnosing a read
    /// that timed out.
    #[must_use]
    pub fn unconsumed(&self) -> &[u8] {
        self.output.pending()
    }

    /// Writes `bytes` as terminal input.
    ///
    /// # Errors
    ///
    /// [`ProbeError::Io`] when the write fails or the writer was handed off.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), ProbeError> {
        let io_error = |source| ProbeError::Io {
            operation: PtyOperation::Write,
            source,
        };
        let writer = self.writer.as_mut().ok_or_else(|| {
            io_error(io::Error::new(
                io::ErrorKind::NotConnected,
                "the input writer was handed off",
            ))
        })?;
        writer
            .write_all(bytes)
            .and_then(|()| writer.flush())
            .map_err(io_error)
    }

    pub(super) const fn take_writer(&mut self) -> Option<Box<dyn Write + Send>> {
        self.writer.take()
    }

    /// Changes the terminal grid.
    ///
    /// # Errors
    ///
    /// [`ProbeError::Backend`] when the backend rejects the size.
    pub fn resize(&self, grid: Grid) -> Result<(), ProbeError> {
        self.master
            .resize(grid.pty_size())
            .map_err(|error| backend(PtyOperation::Resize)(error.into()))
    }

    /// The grid the backend currently reports.
    ///
    /// # Errors
    ///
    /// [`ProbeError::Backend`] when the size cannot be read.
    pub fn grid(&self) -> Result<Grid, ProbeError> {
        let size = self
            .master
            .get_size()
            .map_err(|error| backend(PtyOperation::ReadSize)(error.into()))?;
        Ok(Grid::new(size.cols, size.rows))
    }

    /// Asks the backend to terminate the program (SIGHUP on Unix,
    /// `TerminateProcess` on Windows).
    ///
    /// # Errors
    ///
    /// [`ProbeError::Io`] when the signal cannot be delivered.
    pub fn kill(&mut self) -> Result<(), ProbeError> {
        self.killer.kill().map_err(|source| ProbeError::Io {
            operation: PtyOperation::Kill,
            source,
        })
    }

    /// Consumes output up to and including `needle` and returns those bytes.
    ///
    /// # Errors
    ///
    /// [`ProbeError::Timeout`] after `bound`, [`ProbeError::OutputEnded`] when
    /// the stream ends first and [`ProbeError::OutputLimitExceeded`] when more
    /// than 64 MiB arrive without the marker.
    pub fn read_until(&mut self, needle: &[u8], bound: Duration) -> Result<Vec<u8>, ProbeError> {
        self.output.read_until(needle, bound)
    }

    /// Consumes all output until the stream ends.
    ///
    /// # Errors
    ///
    /// [`ProbeError::Timeout`] when the stream is still open after `bound`.
    pub fn drain_to_end(&mut self, bound: Duration) -> Result<(Vec<u8>, StreamEnd), ProbeError> {
        self.output.drain_to_end(bound)
    }

    /// Waits for the program to exit.
    ///
    /// # Errors
    ///
    /// [`ProbeError::Timeout`] after `bound`; [`ProbeError::Io`] when the
    /// backend cannot collect the status.
    pub fn wait_exit(&mut self, bound: Duration) -> Result<ExitRecord, ProbeError> {
        if let Some(record) = &self.exited {
            return Ok(record.clone());
        }
        let wait_error = |source| ProbeError::Io {
            operation: PtyOperation::Wait,
            source,
        };
        let status = match self.exit.recv_timeout(bound) {
            Ok(status) => status.map_err(wait_error)?,
            Err(RecvTimeoutError::Timeout) => return Err(ProbeError::Timeout { waited: bound }),
            Err(RecvTimeoutError::Disconnected) => {
                return Err(wait_error(io::Error::other(
                    "the waiter ended without a status",
                )));
            }
        };
        let record = ExitRecord::from(&status);
        self.exited = Some(record.clone());
        Ok(record)
    }

    /// Waits for exit and end of output, then joins the session's threads.
    ///
    /// # Errors
    ///
    /// As [`Self::wait_exit`] and [`Self::drain_to_end`].
    pub fn finish(mut self, bound: Duration) -> Result<(ExitRecord, StreamEnd), ProbeError> {
        let exit = self.wait_exit(bound)?;
        let (_tail, end) = self.drain_to_end(bound)?;
        for thread in mem::take(&mut self.threads) {
            thread.join().map_err(|_panic| ProbeError::Io {
                operation: PtyOperation::StartThread,
                source: io::Error::other("a session thread panicked"),
            })?;
        }
        Ok((exit, end))
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        if self.exited.is_none() && self.kill().is_err() {
            tracing::debug!(pid = ?self.pid, "the program had already exited when its session was dropped");
        }
    }
}

//! A program that never reads its input must stay killable while a writer is
//! blocked on the full input queue.

use std::io::{self, Write};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::CommandBuilder;
use serde::Serialize;

use super::error::{ProbeError, PtyOperation};
use super::measure::{ByteCount, Micros};
use super::session::{ExitRecord, Grid, PtySession};

/// Input is written as complete lines: in canonical mode a line longer than
/// the line discipline's buffer is truncated instead of blocking the writer.
const LINE: &[u8; 64] = b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n";
const CHUNK_LINES: usize = 64;
const CHUNKS: u64 = 256;
/// No progress for this long counts as a writer blocked on the input queue.
const QUIET: Duration = Duration::from_millis(250);

enum WriterEvent {
    Progress(u64),
    Done(io::Result<()>),
}

/// What the blocked writer did after the program was killed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WriterAfterKill {
    /// The writer finished; `error` names the OS error kind when it failed.
    Released {
        /// OS error kind of the failed write, if any.
        error: Option<String>,
    },
    /// The writer was still blocked when the probe gave up on it; its thread
    /// is left blocked in the kernel.
    StillBlocked,
}

/// Measured behavior of a program that never reads its input.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct StallReport {
    /// Input the writer attempted.
    pub attempted: ByteCount,
    /// Input accepted before the writer stopped making progress.
    pub accepted_before_kill: ByteCount,
    /// Whether the writer was blocked when the kill was sent.
    pub writer_blocked: bool,
    /// Whether the backend's own kill ended the program within the bound.
    pub backend_kill_ended_program: bool,
    /// The exit finally collected.
    pub exit: ExitRecord,
    /// From the first kill to the collected exit.
    pub kill_to_exit: Micros,
    /// What the writer did after the program died.
    pub writer: WriterAfterKill,
}

#[cfg(unix)]
fn program() -> CommandBuilder {
    let mut command = CommandBuilder::new("sh");
    command.args([
        "-c",
        r#"trap "" HUP; printf 'STALL-READY\n'; exec sleep 600"#,
    ]);
    command
}

#[cfg(windows)]
fn program() -> CommandBuilder {
    let mut command = CommandBuilder::new("cmd.exe");
    command.args(["/d", "/c", "echo STALL-READY& ping -n 600 127.0.0.1 >NUL"]);
    command
}

/// Fills the input of a program that never reads it, then kills the program.
///
/// The backend's kill comes first (SIGHUP on Unix, which this program
/// ignores), then SIGKILL on Unix when the program survived it.
///
/// # Errors
///
/// Any session error, or [`ProbeError::Timeout`] when even the forced kill
/// leaves the program running for `bound`.
pub fn probe_stalled_writer(bound: Duration) -> Result<StallReport, ProbeError> {
    let mut session = PtySession::spawn(program(), Grid::new(80, 24))?;
    session.read_until(b"STALL-READY", bound)?;
    let writer = session.take_writer().ok_or_else(|| ProbeError::Io {
        operation: PtyOperation::Write,
        source: io::Error::new(
            io::ErrorKind::NotConnected,
            "the input writer was handed off",
        ),
    })?;
    let events = spawn_writer(writer)?;
    let (accepted, done) = watch_progress(&events);
    let killed_at = Instant::now();
    session.kill()?;
    let backend_kill = session.wait_exit(bound.min(Duration::from_secs(1)));
    let backend_kill_ended_program = backend_kill.is_ok();
    let exit = match backend_kill {
        Ok(exit) => exit,
        Err(ProbeError::Timeout { .. }) => force(&mut session, bound)?,
        Err(other) => return Err(other),
    };
    let kill_to_exit = Micros::from_duration(killed_at.elapsed());
    let writer_blocked = done.is_none();
    let outcome = done.or_else(|| await_done(&events, Duration::from_secs(1)));
    let writer = outcome.map_or(WriterAfterKill::StillBlocked, |outcome| {
        WriterAfterKill::Released {
            error: outcome.err().map(|error| error.kind().to_string()),
        }
    });
    Ok(StallReport {
        attempted: ByteCount::new(CHUNKS.saturating_mul(
            u64::try_from(LINE.len().saturating_mul(CHUNK_LINES)).unwrap_or(u64::MAX),
        )),
        accepted_before_kill: ByteCount::new(accepted),
        writer_blocked,
        backend_kill_ended_program,
        exit,
        kill_to_exit,
        writer,
    })
}

#[cfg(unix)]
fn force(session: &mut PtySession, bound: Duration) -> Result<ExitRecord, ProbeError> {
    let pid = session.pid().ok_or(ProbeError::OutputEnded)?;
    super::process::force_kill(pid)?;
    session.wait_exit(bound)
}

#[cfg(windows)]
fn force(session: &mut PtySession, bound: Duration) -> Result<ExitRecord, ProbeError> {
    session.wait_exit(bound)
}

fn spawn_writer(mut writer: Box<dyn Write + Send>) -> Result<Receiver<WriterEvent>, ProbeError> {
    let (sender, receiver) = channel();
    let chunk = LINE.repeat(CHUNK_LINES);
    thread::Builder::new()
        .name("probe-stalled-writer".into())
        .spawn(move || {
            let mut written = 0_u64;
            for _ in 0..CHUNKS {
                if let Err(error) = writer.write_all(&chunk) {
                    return sender.send(WriterEvent::Done(Err(error)));
                }
                written = written.saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
                if sender.send(WriterEvent::Progress(written)).is_err() {
                    return Ok(());
                }
            }
            sender.send(WriterEvent::Done(writer.flush()))
        })
        .map_err(|source| ProbeError::Io {
            operation: PtyOperation::StartThread,
            source,
        })?;
    Ok(receiver)
}

/// Collects progress until the writer goes quiet or finishes.
fn watch_progress(events: &Receiver<WriterEvent>) -> (u64, Option<io::Result<()>>) {
    let mut accepted = 0;
    loop {
        match events.recv_timeout(QUIET) {
            Ok(WriterEvent::Progress(written)) => accepted = written,
            Ok(WriterEvent::Done(outcome)) => return (accepted, Some(outcome)),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                return (accepted, None);
            }
        }
    }
}

fn await_done(events: &Receiver<WriterEvent>, bound: Duration) -> Option<io::Result<()>> {
    let deadline = Instant::now().checked_add(bound)?;
    loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(WriterEvent::Progress(_)) => {}
            Ok(WriterEvent::Done(outcome)) => return Some(outcome),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return None,
        }
    }
}

//! Stalled-writer fixture for the native platform probe (`probe_stalled_writer`).
//!
//! `stall` fills the input of a pseudo-terminal program that never reads and
//! ignores SIGHUP, kills it with the backend's kill and then (on Unix) SIGKILL,
//! and observes whether the blocked writer is released. It prints
//! `PROGRAM <pid>` once the program runs and one `STALL <json>` report, then
//! blocks on stdin: the probe SIGKILLs and reaps this process, so a writer
//! still blocked in the kernel never outlives the probe.

use std::io::{self, Read, Write};
use std::process::ExitCode;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde_json::json;
use thiserror::Error;

const SIZE: PtySize = PtySize {
    rows: 24,
    cols: 80,
    pixel_width: 0,
    pixel_height: 0,
};
const READY: &[u8] = b"STALL-READY";
/// Input is written as complete lines: in canonical mode a longer line is
/// truncated by the line discipline instead of blocking the writer.
const LINE: &[u8; 64] = b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n";
const CHUNK_LINES: usize = 64;
const CHUNKS: u64 = 256;
/// No progress for this long counts as a writer blocked on the input queue.
const QUIET: Duration = Duration::from_millis(250);
const STEP: Duration = Duration::from_secs(10);
const AFTER_KILL: Duration = Duration::from_secs(1);

#[derive(Debug, Error)]
enum FixtureError {
    #[error("expected the single argument `stall`")]
    Usage,
    #[error("fixture I/O failed")]
    Io(#[from] io::Error),
    #[error("the pseudo-terminal backend failed")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("the program did not print its readiness marker in time")]
    NotReady,
    #[error("the program did not exit after the kills")]
    NoExit,
}

impl FixtureError {
    const fn kind(&self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::Io(_) => "io_failed",
            Self::Backend(_) => "backend_failed",
            Self::NotReady => "timeout",
            Self::NoExit => "no_exit",
        }
    }

    const fn hint(&self) -> &'static str {
        match self {
            Self::Usage => "run the fixture through probe_stalled_writer",
            Self::Io(_) | Self::Backend(_) => "the fixture host is broken; rerun the probe",
            Self::NotReady => "the terminal withheld the program's output",
            Self::NoExit => "the program survived every kill; the probe reaps it by PID",
        }
    }
}

fn backend(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> FixtureError {
    FixtureError::Backend(error.into())
}

fn main() -> ExitCode {
    let outcome = match std::env::args().nth(1).as_deref() {
        Some("stall") => stall(),
        Some(_) | None => Err(FixtureError::Usage),
    };
    let Err(error) = outcome else {
        return ExitCode::SUCCESS;
    };
    let line = format!(
        "{}: {error} ({}): {:?}\n",
        error.kind(),
        error.hint(),
        std::error::Error::source(&error)
    );
    if io::stderr().write_all(line.as_bytes()).is_err() {
        return ExitCode::from(3);
    }
    ExitCode::FAILURE
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

fn stall() -> Result<(), FixtureError> {
    let pair = native_pty_system().openpty(SIZE).map_err(backend)?;
    let mut child = pair.slave.spawn_command(program()).map_err(backend)?;
    drop(pair.slave);
    let pid = child.process_id();
    say(&format!(
        "PROGRAM {}",
        pid.map_or_else(String::new, |pid| pid.to_string())
    ))?;
    let ready = watch_output(pair.master.try_clone_reader().map_err(backend)?)?;
    ready
        .recv_timeout(STEP)
        .map_err(|_late| FixtureError::NotReady)?;
    let progress = spawn_writer(pair.master.take_writer().map_err(backend)?)?;
    let (accepted, done) = watch_progress(&progress);
    let mut killer = child.clone_killer();
    let (exited_sender, exited) = channel();
    thread::Builder::new().spawn(move || drop(exited_sender.send(child.wait())))?;
    let killed_at = Instant::now();
    let backend_kill = killer.kill();
    let mut status = exited.recv_timeout(AFTER_KILL);
    let backend_kill_ended_program = status.is_ok();
    if status.is_err() {
        force_kill(pid)?;
        status = exited.recv_timeout(STEP);
    }
    let status = status.map_err(|_alive| FixtureError::NoExit)??;
    let kill_to_exit = killed_at.elapsed();
    let writer_blocked = done.is_none();
    let writer = done.or_else(|| await_done(&progress)).map_or_else(
        || json!("still_blocked"),
        |outcome| json!({ "released": { "error": outcome.err().map(|error| error.kind().to_string()) } }),
    );
    let report = json!({
        "attempted": CHUNKS.saturating_mul(u64::try_from(LINE.len().saturating_mul(CHUNK_LINES)).unwrap_or(u64::MAX)).to_string(),
        "accepted_before_kill": accepted.to_string(),
        "writer_blocked": writer_blocked,
        "backend_kill_ended_program": backend_kill_ended_program,
        "backend_kill_error": backend_kill.err().map(|error| error.to_string()),
        "exit": { "code": status.exit_code(), "signal": status.signal() },
        "kill_to_exit": u64::try_from(kill_to_exit.as_micros()).unwrap_or(u64::MAX).to_string(),
        "writer": writer,
    });
    say(&format!("STALL {report}"))?;
    io::stdin().lock().read_to_end(&mut Vec::new())?;
    Ok(())
}

fn say(line: &str) -> Result<(), FixtureError> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(line.as_bytes())?;
    stdout.write_all(b"\n")?;
    Ok(stdout.flush()?)
}

#[cfg(unix)]
fn force_kill(pid: Option<u32>) -> Result<(), FixtureError> {
    let pid = pid.ok_or(FixtureError::NoExit)?.to_string();
    let status = std::process::Command::new("kill")
        .args(["-KILL", &pid])
        .stderr(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(FixtureError::NoExit)
    }
}

#[cfg(windows)]
const fn force_kill(_pid: Option<u32>) -> Result<(), FixtureError> {
    Err(FixtureError::NoExit)
}

/// Discards output and signals once the readiness marker has been seen.
fn watch_output(mut output: Box<dyn Read + Send>) -> Result<Receiver<()>, FixtureError> {
    let (sender, ready) = channel();
    thread::Builder::new().spawn(move || {
        let mut seen = Vec::new();
        let mut announced = false;
        let mut buffer = [0_u8; 4096];
        while let Ok(read @ 1..) = output.read(&mut buffer) {
            if !announced && seen.len() < 4096 {
                seen.extend_from_slice(buffer.get(..read).unwrap_or_default());
                announced = seen.windows(READY.len()).any(|window| window == READY);
                if announced && sender.send(()).is_err() {
                    return;
                }
            }
        }
    })?;
    Ok(ready)
}

enum WriterEvent {
    Progress(u64),
    Done(io::Result<()>),
}

fn spawn_writer(mut writer: Box<dyn Write + Send>) -> Result<Receiver<WriterEvent>, FixtureError> {
    let (sender, events) = channel();
    let chunk = LINE.repeat(CHUNK_LINES);
    thread::Builder::new().spawn(move || {
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
    })?;
    Ok(events)
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

fn await_done(events: &Receiver<WriterEvent>) -> Option<io::Result<()>> {
    let deadline = Instant::now().checked_add(AFTER_KILL)?;
    loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(WriterEvent::Progress(_)) => {}
            Ok(WriterEvent::Done(outcome)) => return Some(outcome),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return None,
        }
    }
}

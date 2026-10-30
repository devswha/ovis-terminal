//! Holder fixture for the native platform probe (`probes/platform/platform_probe.rs`).
//!
//! - `launch` runs in a login terminal: it starts `serve` in a new process
//!   group, relays its `HELD holder=<pid> program=<pid>` line, then reads the
//!   terminal until the login ends.
//! - `serve` owns a pseudo-terminal running `cat` and pumps it until `cat` exits.
//! - `openpty` opens one pseudo-terminal and, when the platform refuses, writes
//!   the holder-protocol `SpawnFailed` frame a holder would send.

use std::io::{self, Write};
use std::process::ExitCode;

#[cfg(unix)]
use ovis_terminal_holder::protocol::FrameError;

#[cfg(unix)]
#[derive(Debug, thiserror::Error)]
enum FixtureError {
    #[error("expected one argument: launch, serve or openpty (Unix only)")]
    Usage,
    #[error("fixture I/O failed")]
    Io(#[from] io::Error),
    #[error("the pseudo-terminal backend failed")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("the holder exited before reporting its processes")]
    NoReport,
    #[error("the backend reported no program PID")]
    NoPid,
    #[error("the holder frame could not be encoded")]
    Frame(#[from] FrameError),
    #[error("a pseudo-terminal opened although descriptors were exhausted")]
    OpenedUnderLimit,
}

#[cfg(unix)]
impl FixtureError {
    const fn kind(&self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::Io(_) => "io_failed",
            Self::Backend(_) => "backend_failed",
            Self::NoReport => "no_report",
            Self::NoPid => "no_pid",
            Self::Frame(_) => "frame_failed",
            Self::OpenedUnderLimit => "opened_under_limit",
        }
    }

    const fn hint(&self) -> &'static str {
        match self {
            Self::Usage => "run the fixture through the platform probe",
            Self::Io(_) | Self::Backend(_) | Self::Frame(_) => {
                "the fixture host is broken; rerun the probe"
            }
            Self::NoReport | Self::NoPid => {
                "the holder died before reporting; inspect the probe host"
            }
            Self::OpenedUnderLimit => "run openpty mode under `ulimit -n 4`",
        }
    }
}

#[cfg(not(unix))]
fn main() -> ExitCode {
    if io::stderr()
        .write_all(b"usage: the holder fixture runs on Unix only\n")
        .is_err()
    {
        return ExitCode::from(3);
    }
    ExitCode::from(2)
}

#[cfg(unix)]
fn main() -> ExitCode {
    match run(std::env::args().nth(1).as_deref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let line = format!("{}: {error} ({})\n", error.kind(), error.hint());
            if io::stderr().write_all(line.as_bytes()).is_err() {
                return ExitCode::from(3);
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(unix)]
fn run(mode: Option<&str>) -> Result<(), FixtureError> {
    match mode {
        Some("launch") => launch(),
        Some("serve") => serve(),
        Some("openpty") => openpty(),
        Some(_) | None => Err(FixtureError::Usage),
    }
}

#[cfg(unix)]
const SIZE: portable_pty::PtySize = portable_pty::PtySize {
    rows: 24,
    cols: 80,
    pixel_width: 0,
    pixel_height: 0,
};

#[cfg(unix)]
fn backend(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> FixtureError {
    FixtureError::Backend(error.into())
}

#[cfg(unix)]
fn launch() -> Result<(), FixtureError> {
    use std::io::{BufRead, BufReader, Read};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut holder = Command::new(std::env::current_exe()?)
        .arg("serve")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let report = holder.stdout.take().ok_or(FixtureError::NoReport)?;
    let mut line = String::new();
    if BufReader::new(report).read_line(&mut line)? == 0 {
        return Err(FixtureError::NoReport);
    }
    let mut stdout = io::stdout().lock();
    stdout.write_all(line.as_bytes())?;
    stdout.flush()?;
    drop(stdout);
    io::stdin().lock().read_to_end(&mut Vec::new())?;
    Ok(())
}

#[cfg(unix)]
fn serve() -> Result<(), FixtureError> {
    use portable_pty::{CommandBuilder, native_pty_system};

    let pair = native_pty_system().openpty(SIZE).map_err(backend)?;
    let mut program = pair
        .slave
        .spawn_command(CommandBuilder::new("cat"))
        .map_err(backend)?;
    drop(pair.slave);
    let mut output = pair.master.try_clone_reader().map_err(backend)?;
    let pid = program.process_id().ok_or(FixtureError::NoPid)?;
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "HELD holder={} program={pid}", std::process::id())?;
    stdout.flush()?;
    drop(stdout);
    io::copy(&mut output, &mut io::sink())?;
    program.wait()?;
    Ok(())
}

#[cfg(unix)]
fn openpty() -> Result<(), FixtureError> {
    use ovis_terminal_holder::protocol::{
        Message, SpawnFailed, SpawnFailureKind, TerminalId, TerminalIncarnation, encode_message,
    };
    use portable_pty::native_pty_system;

    let refusal = match native_pty_system().openpty(SIZE) {
        Ok(_pair) => return Err(FixtureError::OpenedUnderLimit),
        Err(refusal) => refusal,
    };
    let frame = encode_message(&Message::SpawnFailed(SpawnFailed {
        terminal_id: TerminalId::new("probe-openpty".to_owned()),
        terminal_incarnation: TerminalIncarnation::new(1),
        kind: SpawnFailureKind::Unavailable,
    }))?;
    let mut stdout = io::stdout().lock();
    stdout.write_all(&frame)?;
    stdout.flush()?;
    io::stderr().write_all(format!("openpty refused: {refusal:#}\n").as_bytes())?;
    Ok(())
}

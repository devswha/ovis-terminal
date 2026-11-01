//! A PTY whose slave is the controlling terminal of one binary process.

use std::fs::File;
use std::io::{Read as _, Write as _};
use std::process::Stdio;
use std::time::Duration;

use rustix::termios::{self, Winsize};
use tokio::io::AsyncReadExt as _;
use tokio::sync::mpsc;

use crate::engine::transport::Endpoint;
use crate::error::RunnerError;
use crate::evidence::{CapturedText, ProcessReport};
use crate::ids::Decimal;

/// A finished client: exit code, stderr bytes, whether the outer terminal settings are
/// the original ones, and its process record.
pub(crate) type Exited = (Option<i32>, Vec<u8>, bool, ProcessReport);

/// Bound on the client exiting once its exit is awaited.
const EXIT_BOUND: Duration = Duration::from_secs(30);

fn io(operation: &'static str) -> impl FnOnce(std::io::Error) -> RunnerError {
    move |source| RunnerError::Io { operation, source }
}

fn errno(operation: &'static str) -> impl FnOnce(rustix::io::Errno) -> RunnerError {
    move |errno| RunnerError::Io {
        operation,
        source: errno.into(),
    }
}

/// The outer terminal of one client and everything it drew.
#[derive(Debug)]
pub(crate) struct OuterPty {
    master: File,
    slave_path: std::path::PathBuf,
    original: String,
    child: tokio::process::Child,
    argv: Vec<String>,
    output: mpsc::UnboundedReceiver<Vec<u8>>,
    seen: Vec<u8>,
}

/// The tty's settings through a fresh descriptor: Darwin revokes the descriptors of a
/// controlling terminal when its session leader exits, but the settings belong to the
/// tty, which lives on while the master is open.
fn settings_of(path: &std::path::Path) -> Result<String, RunnerError> {
    use rustix::fs::{Mode, OFlags};
    let fresh = rustix::fs::open(path, OFlags::RDWR | OFlags::NOCTTY, Mode::empty())
        .map_err(errno("reopen the PTY slave"))?;
    settings(&File::from(fresh))
}

fn settings(slave: &File) -> Result<String, RunnerError> {
    Ok(format!(
        "{:?}",
        termios::tcgetattr(slave).map_err(errno("read terminal settings"))?
    ))
}

const fn winsize(cols: u16, rows: u16) -> Winsize {
    Winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

impl OuterPty {
    /// Starts `endpoint.binary <args>` with the bearer, on a fresh `cols`x`rows` PTY.
    pub(crate) fn open(
        endpoint: &Endpoint,
        (args, bearer): (&[String], Option<&str>),
        (cols, rows): (u16, u16),
    ) -> Result<Self, RunnerError> {
        use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
        let master = File::from(
            openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).map_err(errno("open a PTY"))?,
        );
        grantpt(&master).map_err(errno("grant the PTY"))?;
        unlockpt(&master).map_err(errno("unlock the PTY"))?;
        let name = ptsname(&master, Vec::new()).map_err(errno("name the PTY"))?;
        let slave_path = std::path::PathBuf::from(
            <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(name.as_bytes()),
        );
        let slave = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&slave_path)
            .map_err(io("open the PTY slave"))?;
        termios::tcsetwinsize(&master, winsize(cols, rows)).map_err(errno("size the PTY"))?;
        let original = settings(&slave)?;
        let mut command = tokio::process::Command::new(&endpoint.binary);
        command.args(args);
        endpoint.isolate(&mut command);
        if let Some(bearer) = bearer {
            command.env("OVIS_TERMINAL_CREDENTIAL", bearer);
        }
        command
            .stdin(slave.try_clone().map_err(io("share the PTY slave"))?)
            .stdout(slave.try_clone().map_err(io("share the PTY slave"))?)
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // SAFETY: between fork and exec the closure only calls the async-signal-safe setsid
        // and the TIOCSCTTY ioctl on its own stdin, so the PTY becomes its controlling terminal.
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(rustix::stdio::stdin())?;
                Ok(())
            });
        }
        let child = command.spawn().map_err(|source| RunnerError::Spawn {
            program: endpoint.binary.clone(),
            source,
        })?;
        let (sender, output) = mpsc::unbounded_channel();
        let mut reader = master.try_clone().map_err(io("share the PTY master"))?;
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            while let Ok(read) = reader.read(&mut buffer) {
                let chunk = buffer.get(..read).map(<[u8]>::to_vec).unwrap_or_default();
                if read == 0 || sender.send(chunk).is_err() {
                    break;
                }
            }
        });
        let mut command_line = vec![endpoint.binary.display().to_string()];
        command_line.extend(args.iter().cloned());
        Ok(Self {
            master,
            slave_path,
            original,
            child,
            argv: command_line,
            output,
            seen: Vec::new(),
        })
    }

    pub(crate) fn type_bytes(&mut self, bytes: &[u8]) -> Result<(), RunnerError> {
        self.master.write_all(bytes).map_err(io("type on the PTY"))
    }

    pub(crate) fn resize(&self, (cols, rows): (u16, u16)) -> Result<(), RunnerError> {
        termios::tcsetwinsize(&self.master, winsize(cols, rows)).map_err(errno("resize the PTY"))
    }

    /// Waits, bounded by `within`, for `needle` in everything drawn so far.
    pub(crate) async fn shows(&mut self, needle: &str, within: Duration) -> bool {
        let found = |seen: &[u8]| {
            seen.windows(needle.len())
                .any(|window| window == needle.as_bytes())
        };
        let wait = async {
            while !found(&self.seen) {
                match self.output.recv().await {
                    Some(chunk) => self.seen.extend(chunk),
                    None => return false,
                }
            }
            true
        };
        tokio::time::timeout(within, wait).await.unwrap_or(false)
    }

    /// Whether `needle` is in everything delivered so far, without waiting for more.
    pub(crate) fn delivered_contains(&mut self, needle: &str) -> bool {
        while let Ok(chunk) = self.output.try_recv() {
            self.seen.extend(chunk);
        }
        self.seen
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    }

    /// Awaits the exit: its code, stderr bytes, whether the settings are the original ones,
    /// and the process record with the drawn screen as its stdout.
    pub(crate) async fn exit(mut self) -> Result<Exited, RunnerError> {
        let mut stderr = Vec::new();
        if let Some(mut pipe) = self.child.stderr.take() {
            pipe.read_to_end(&mut stderr)
                .await
                .map_err(io("read the client's stderr"))?;
        }
        let status = tokio::time::timeout(EXIT_BOUND, self.child.wait())
            .await
            .map_err(|_| RunnerError::Timeout {
                waiting_for: "the attach client to exit",
                bound_ms: u64::try_from(EXIT_BOUND.as_millis()).unwrap_or(u64::MAX),
            })?
            .map_err(io("wait for the attach client"))?;
        while let Ok(chunk) = self.output.try_recv() {
            self.seen.extend(chunk);
        }
        let restored = settings_of(&self.slave_path)? == self.original;
        let report = ProcessReport {
            role: "attach",
            argv: self.argv,
            pid: self.child.id().map(|pid| Decimal(u64::from(pid))),
            exit_code: status.code(),
            stdout: CapturedText::of(&self.seen),
            stderr: CapturedText::of(&stderr),
        };
        Ok((status.code(), stderr, restored, report))
    }
}

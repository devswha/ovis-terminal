//! The outer terminal of one attach client: a PTY the test types into and reads back.

use std::fs::File;
use std::io::{Read as _, Write as _};
use std::os::unix::process::CommandExt as _;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use rustix::termios::{self, Winsize};
use serde_json::Value;

use super::{Served, TestResult, WAIT};

/// An outer PTY whose slave is the controlling terminal of one attach client.
#[derive(Debug)]
pub struct Outer {
    /// The PTY master: what the test types and reads.
    pub master: File,
    slave_path: PathBuf,
    output: mpsc::Receiver<Vec<u8>>,
    seen: Vec<u8>,
    original: String,
    /// The attach client process.
    pub client: Child,
}

fn settings(file: &File) -> TestResult<String> {
    Ok(format!("{:?}", termios::tcgetattr(file)?))
}

/// The tty's settings through a fresh descriptor: Darwin revokes the descriptors of a
/// controlling terminal when its session leader exits, but the settings belong to the
/// tty, which lives on while the master is open.
fn settings_of(path: &std::path::Path) -> TestResult<String> {
    use rustix::fs::{Mode, OFlags};
    let fresh = File::from(rustix::fs::open(
        path,
        OFlags::RDWR | OFlags::NOCTTY,
        Mode::empty(),
    )?);
    settings(&fresh)
}

impl Outer {
    /// Starts the binary with `args` on a fresh 100x30 outer PTY as its controlling terminal.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    pub fn attach(served: &Served, args: &[&str]) -> TestResult<Self> {
        let master = File::from(rustix::pty::openpt(
            rustix::pty::OpenptFlags::RDWR | rustix::pty::OpenptFlags::NOCTTY,
        )?);
        rustix::pty::grantpt(&master)?;
        rustix::pty::unlockpt(&master)?;
        let name = rustix::pty::ptsname(&master, Vec::new())?;
        let slave_path = PathBuf::from(name.to_str()?);
        let slave = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&slave_path)?;
        termios::tcsetwinsize(
            &master,
            Winsize {
                ws_row: 30,
                ws_col: 100,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )?;
        let original = settings(&slave)?;
        let mut command = served.command(args);
        command
            .stdin(slave.try_clone()?)
            .stdout(slave.try_clone()?)
            .stderr(Stdio::piped());
        // SAFETY: between fork and exec the closure only calls async-signal-safe setsid and
        // the TIOCSCTTY ioctl on its own stdin, making the PTY slave its controlling terminal.
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(rustix::stdio::stdin())?;
                Ok(())
            });
        }
        let client = command.spawn()?;
        let (sender, output) = mpsc::channel();
        let mut reader = master.try_clone()?;
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0
                    || sender
                        .send(buffer.get(..read).map(<[u8]>::to_vec).unwrap_or_default())
                        .is_err()
                {
                    break;
                }
            }
        });
        Ok(Self {
            master,
            slave_path,
            output,
            seen: Vec::new(),
            original,
            client,
        })
    }

    /// Types `bytes` on the outer terminal.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    pub fn type_bytes(&mut self, bytes: &[u8]) -> TestResult {
        Ok(self.master.write_all(bytes)?)
    }

    /// Waits, bounded, until `needle` appears in what the client drew on the outer PTY.
    pub fn shows(&mut self, needle: &str, within: Duration) -> bool {
        let Some(deadline) = Instant::now().checked_add(within) else {
            return false;
        };
        loop {
            if self
                .seen
                .windows(needle.len())
                .any(|window| window == needle.as_bytes())
            {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.output.recv_timeout(left) {
                Ok(chunk) => self.seen.extend(chunk),
                Err(_) => return false,
            }
        }
    }

    /// Everything the client drew on the outer PTY, read until the client closed its side
    /// of the terminal by exiting.
    ///
    /// # Errors
    ///
    /// Fails when the client still holds the terminal open after `within`.
    pub fn drawn_until_closed(&mut self, within: Duration) -> TestResult<Vec<u8>> {
        let deadline = Instant::now().checked_add(within).ok_or("deadline")?;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.output.recv_timeout(left) {
                Ok(chunk) => self.seen.extend(chunk),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(self.seen.clone()),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err("the client kept the outer terminal open".into());
                }
            }
        }
    }

    /// Waits for the client to exit; returns its code and parsed stderr record.
    ///
    /// # Errors
    ///
    /// Fails when a process, socket or file step fails.
    ///
    /// # Panics
    ///
    /// Panics when the binary's behaviour differs from the expectation.
    pub fn exited(self) -> TestResult<(Option<i32>, Value, bool)> {
        let Self {
            master,
            slave_path,
            original,
            mut client,
            ..
        } = self;
        let pid = client.id();
        let (sender, exited) = mpsc::channel();
        std::thread::spawn(move || {
            let mut stderr = String::new();
            if let Some(mut pipe) = client.stderr.take() {
                pipe.read_to_string(&mut stderr).ok();
            }
            sender
                .send((client.wait().map(|status| status.code()), stderr))
                .ok();
        });
        let Ok((status, stderr)) = exited.recv_timeout(WAIT) else {
            let pid = rustix::process::Pid::from_raw(i32::try_from(pid)?).ok_or("pid")?;
            assert!(pid.as_raw_nonzero().get() > 1);
            rustix::process::kill_process(pid, rustix::process::Signal::KILL)?;
            return Err("the attach client did not exit".into());
        };
        let record =
            serde_json::from_str(stderr.lines().last().unwrap_or("null")).unwrap_or(Value::Null);
        let restored = settings_of(&slave_path)? == original;
        drop(master);
        Ok((status?, record, restored))
    }
}

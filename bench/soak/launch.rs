//! Control boots as the harness sees them: started from this executable, commanded over
//! stdin, crashed with SIGKILL or awaited after a handover.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::time::Duration;

use serde::Deserialize;

use super::error::SoakError;
use super::process::{self, Started};
use super::wire::BootMode;

/// Bound on a control boot announcing its socket.
const ANNOUNCE: Duration = Duration::from_secs(30);
/// Bound on a handed-over or stopped boot exiting.
const EXIT: Duration = Duration::from_secs(60);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Listening {
    listening: PathBuf,
}

#[derive(Debug)]
pub(super) struct ControlProcess {
    pub(super) socket: PathBuf,
    stdin: Option<ChildStdin>,
    started: Started,
}

impl ControlProcess {
    /// Starts boot `boot` of the control process rooted at `root`.
    pub(super) fn start(
        exe: &Path,
        (root, logs): (&Path, &Path),
        (mode, boot): (BootMode, u64),
    ) -> Result<Self, SoakError> {
        let stderr = std::fs::File::create(logs.join(format!("control-{boot}.stderr")))
            .map_err(SoakError::io("create a control log"))?;
        let mode_name = match mode {
            BootMode::Fresh => "fresh",
            BootMode::Crash => "crash",
            BootMode::Handover => "handover",
        };
        let mut child = Command::new(exe)
            .arg("control")
            .arg("--root")
            .arg(root)
            .args(["--boot-mode", mode_name, "--boot", &boot.to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .map_err(SoakError::io("start a control boot"))?;
        let stdin = child.stdin.take();
        let stdout_log = logs.join(format!("control-{boot}.stdout"));
        let started = process::first_line(
            child,
            (ANNOUNCE, "control socket announcement"),
            &stdout_log,
        )?;
        let listening: Listening = serde_json::from_str(started.first_line.trim())?;
        Ok(Self {
            socket: listening.listening,
            stdin,
            started,
        })
    }

    pub(super) const fn pid(&self) -> u32 {
        self.started.pid
    }

    /// Writes one command line; `close` ends stdin after it.
    pub(super) fn command(&mut self, line: &str, close: bool) -> Result<(), SoakError> {
        let stdin = self.stdin.as_mut().ok_or(SoakError::ControlGone {
            waiting_for: "a command channel",
        })?;
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .and_then(|()| stdin.flush())
            .map_err(SoakError::io("command the control process"))?;
        if close {
            self.stdin = None;
        }
        Ok(())
    }

    /// The signalled crash: SIGKILL to the captured PID.
    pub(super) fn crash(self) -> Result<(), SoakError> {
        let Self { stdin, started, .. } = self;
        drop(stdin);
        started.kill().map(drop)
    }

    /// Awaits the exit of a boot told to hand over or stop; its exit code.
    pub(super) fn exit(self) -> Result<Option<i32>, SoakError> {
        let Self { stdin, started, .. } = self;
        drop(stdin);
        Ok(started.wait((EXIT, "control boot exit"))?.code())
    }
}

//! One process of the second private runtime: this runner re-run as `fixture --peer`.
//!
//! Only the pid captured at its own spawn is ever signalled, after checking it is above 1.

use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use ovis_terminal_protocol::errors::BoundaryError;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt as _, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};

use super::super::machines::internal;
use super::super::serve::at;
use crate::error::RunnerError;

const BOUND: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
struct ReadyLine {
    ready: Ready,
}

#[derive(Debug, Deserialize)]
struct Ready {
    socket: PathBuf,
    bearer: String,
}

#[derive(Debug)]
pub(super) struct Process {
    child: Child,
    stdin: Option<ChildStdin>,
    _stdout: Lines<BufReader<ChildStdout>>,
    pid: u32,
    pub(super) socket: PathBuf,
    pub(super) bearer: String,
}

impl Process {
    /// Starts the peer runtime in a fresh owner-only `dir` and waits for its ready line.
    pub(super) async fn spawn(runner: &Path, dir: &Path) -> Result<Self, RunnerError> {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(dir)
            .map_err(at("create the peer root"))?;
        let mut child = tokio::process::Command::new(runner)
            .arg("fixture")
            .arg("--root")
            .arg(dir)
            .arg("--peer")
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(at("spawn the peer runtime"))?;
        let pid = child.id().filter(|pid| *pid > 1);
        let pid = pid.ok_or_else(|| at("capture the peer pid")("no usable pid"))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stdout = stdout.ok_or_else(|| at("pipe the peer stdout")("not piped"))?;
        let mut stdout = BufReader::new(stdout).lines();
        let line = tokio::time::timeout(BOUND, stdout.next_line()).await;
        let line = line.map_err(at("await the peer ready line"))?;
        let line = line.map_err(at("read the peer ready line"))?;
        let line = line.ok_or_else(|| at("read the peer ready line")("the peer exited"))?;
        let ready: ReadyLine =
            serde_json::from_str(&line).map_err(at("parse the peer ready line"))?;
        Ok(Self {
            child,
            stdin,
            _stdout: stdout,
            pid,
            socket: ready.ready.socket,
            bearer: ready.ready.bearer,
        })
    }

    /// Sends `signal` (`STOP` or `CONT`) to this process only.
    pub(super) fn signal(&self, signal: &str) -> Result<(), BoundaryError> {
        let status = std::process::Command::new("/bin/kill")
            .arg(format!("-{signal}"))
            .arg(self.pid.to_string())
            .status()
            .map_err(internal)?;
        if status.success() {
            Ok(())
        } else {
            Err(internal(status))
        }
    }

    /// Kills this process by its own handle and reaps it, bounded.
    pub(super) async fn crash(&mut self) -> Result<(), BoundaryError> {
        self.child.start_kill().map_err(internal)?;
        tokio::time::timeout(BOUND, self.child.wait())
            .await
            .map_err(internal)?
            .map_err(internal)?;
        Ok(())
    }

    /// Resumes a stopped process, closes its stdin and reaps it; past the bound it is killed.
    pub(super) async fn stop(mut self) -> Result<(), RunnerError> {
        if matches!(self.child.try_wait(), Ok(None)) {
            self.signal("CONT")
                .map_err(|_| at("resume the peer runtime")("kill -CONT failed"))?;
        }
        drop(self.stdin.take());
        if tokio::time::timeout(BOUND, self.child.wait())
            .await
            .is_err()
        {
            self.child
                .start_kill()
                .map_err(at("kill the peer runtime"))?;
        }
        Ok(())
    }
}

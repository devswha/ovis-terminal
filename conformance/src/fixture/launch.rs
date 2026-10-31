//! The runner's handle on a fixture process: ready on stdout, stop by closing stdin.
//!
//! No signal is ever sent to stop it; only after the bound expires is this one child
//! killed by its own handle, never by a process group or a guessed pid.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader, Lines};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};

use crate::error::RunnerError;
use crate::evidence::{CapturedText, ProcessReport};
use crate::ids::Decimal;

const READY_BOUND: Duration = Duration::from_secs(30);
const STOP_BOUND: Duration = Duration::from_secs(30);

/// The fixture's first stdout line.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadyLine {
    ready: Ready,
}

/// Where the fixture listens and the owner bearer it issued.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ready {
    pub(crate) socket: PathBuf,
    pub(crate) bearer: String,
}

/// A running fixture process owned by one case.
#[derive(Debug)]
pub(crate) struct FixtureProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Lines<BufReader<ChildStdout>>,
    stderr: Option<ChildStderr>,
    argv: Vec<String>,
    pub(crate) ready: Ready,
}

/// Starts `runner fixture --root <root>` and waits, bounded, for its ready line.
pub(crate) async fn start(runner: &Path, root: &Path) -> Result<FixtureProcess, RunnerError> {
    let mut command = tokio::process::Command::new(runner);
    command
        .arg("fixture")
        .arg("--root")
        .arg(root)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|source| RunnerError::Spawn {
        program: runner.to_path_buf(),
        source,
    })?;
    let not_piped = RunnerError::FixtureReport {
        expected: "piped standard streams",
    };
    let stdin = child.stdin.take();
    let stdout = child.stdout.take().ok_or(not_piped)?;
    let stderr = child.stderr.take();
    let mut stdout = BufReader::new(stdout).lines();
    let line = tokio::time::timeout(READY_BOUND, stdout.next_line()).await;
    let not_ready = RunnerError::FixtureReport {
        expected: "a ready line",
    };
    let Ok(Ok(Some(line))) = line else {
        return Err(not_ready);
    };
    let ready: ReadyLine = serde_json::from_str(&line).map_err(|_| not_ready)?;
    Ok(FixtureProcess {
        child,
        stdin,
        stdout,
        stderr,
        argv: vec![
            runner.display().to_string(),
            "fixture".to_owned(),
            "--root".to_owned(),
            root.display().to_string(),
        ],
        ready: ready.ready,
    })
}

impl FixtureProcess {
    /// Closes stdin, reads the serve report and reaps the process.
    pub(crate) async fn stop(mut self) -> (Result<Value, RunnerError>, ProcessReport) {
        drop(self.stdin.take());
        let line = tokio::time::timeout(STOP_BOUND, self.stdout.next_line()).await;
        let report = match line {
            Ok(Ok(Some(line))) => serde_json::from_str::<Value>(&line)
                .ok()
                .and_then(|value| value.get("report").cloned())
                .ok_or(RunnerError::FixtureReport {
                    expected: "a serve report line",
                }),
            Ok(Ok(None) | Err(_)) | Err(_) => Err(RunnerError::FixtureReport {
                expected: "a serve report line",
            }),
        };
        let pid = self.child.id().map(|pid| Decimal(u64::from(pid)));
        let mut notes = Vec::new();
        let exit = match tokio::time::timeout(STOP_BOUND, self.child.wait()).await {
            Ok(Ok(status)) => status.code(),
            Ok(Err(error)) => {
                notes.extend_from_slice(format!("<wait failed: {error}>\n").as_bytes());
                None
            }
            Err(_) => {
                let note = match self.child.start_kill() {
                    Ok(()) => "<killed after the stop bound>\n".to_owned(),
                    Err(error) => format!("<kill after the stop bound failed: {error}>\n"),
                };
                notes.extend_from_slice(note.as_bytes());
                None
            }
        };
        if let Some(mut pipe) = self.stderr.take()
            && !matches!(
                tokio::time::timeout(STOP_BOUND, pipe.read_to_end(&mut notes)).await,
                Ok(Ok(_))
            )
        {
            notes.extend_from_slice(b"<stderr unreadable>");
        }
        let process = ProcessReport {
            role: "fixture",
            argv: self.argv,
            pid,
            exit_code: exit,
            stdout: CapturedText::of(b"<ready and report lines are parsed, not stored: the ready line carries the bearer>"),
            stderr: CapturedText::of(&notes),
        };
        (report, process)
    }
}

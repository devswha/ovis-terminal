//! Holders of the binary under test: `ovis-terminal --json service run --manager
//! detached` per session, in the run's private runtime directory, plus the binary's own
//! read-only `service status` and `service stop` verbs.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use ovis_terminal_probes::platform::{Pid, observe_process};
use ovis_terminal_protocol::domain::unix_service::DiscoveryRecord;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::error::SoakError;
use super::process::{self, Started};

const READY: Duration = Duration::from_secs(15);

/// How every holder and CLI invocation is run: one binary, one private runtime directory.
#[derive(Debug, Clone)]
pub(super) struct HolderEnv {
    pub(super) binary: PathBuf,
    pub(super) runtime: PathBuf,
    pub(super) logs: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadyLine {
    ready: DiscoveryRecord,
}

/// One running holder process.
#[derive(Debug, Serialize)]
pub(super) struct HolderProcess {
    pub(super) session: String,
    pub(super) socket: PathBuf,
    pub(super) start_token: String,
    pub(super) pid: u32,
    #[serde(skip)]
    started: Started,
}

/// One materialized invocation of the binary under test and its result.
#[derive(Debug, Clone, Serialize)]
pub(super) struct CliRun {
    pub(super) argv: Vec<String>,
    pub(super) exit_code: Option<i32>,
    pub(super) stdout: Value,
}

impl HolderEnv {
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .args(args)
            .env_clear()
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .current_dir("/");
        command
    }

    fn log(&self, name: &str) -> PathBuf {
        self.logs.join(name)
    }

    /// Starts the holder of `session` and awaits its ready line.
    pub(super) fn start(&self, session: &str) -> Result<HolderProcess, SoakError> {
        let stderr = std::fs::File::create(self.log(&format!("holder-{session}.stderr")))
            .map_err(SoakError::io("create a holder log"))?;
        let child = self
            .command(&[
                "--json",
                "service",
                "run",
                "--session",
                session,
                "--manager",
                "detached",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .map_err(|source| SoakError::Binary { source })?;
        let stdout_log = self.log(&format!("holder-{session}.stdout"));
        let stderr_log = self.log(&format!("holder-{session}.stderr"));
        let not_ready = |error: SoakError| match error {
            SoakError::ChildExited { .. } => SoakError::HolderNotReady {
                session: session.to_owned(),
                stderr: std::fs::read_to_string(&stderr_log)
                    .unwrap_or_default()
                    .trim()
                    .to_owned(),
            },
            other => other,
        };
        let started = process::first_line(child, (READY, "holder ready line"), &stdout_log)
            .map_err(not_ready)?;
        let ready: ReadyLine = serde_json::from_str(started.first_line.trim()).map_err(|_| {
            SoakError::HolderNotReady {
                session: session.to_owned(),
                stderr: started.first_line.clone(),
            }
        })?;
        let observed = observe_process(Pid::new(started.pid))?;
        Ok(HolderProcess {
            pid: started.pid,
            session: session.to_owned(),
            socket: PathBuf::from(ready.ready.address),
            start_token: observed.start_token.as_str().to_owned(),
            started,
        })
    }

    /// Runs `ovis-terminal --json <args>` once and keeps its JSON result.
    pub(super) fn cli(&self, args: &[&str]) -> Result<CliRun, SoakError> {
        let mut full = vec!["--json"];
        full.extend_from_slice(args);
        let output = self
            .command(&full)
            .stdin(Stdio::null())
            .output()
            .map_err(|source| SoakError::Binary { source })?;
        let shown = if output.stdout.is_empty() {
            &output.stderr
        } else {
            &output.stdout
        };
        let stdout = serde_json::from_slice(shown)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(shown).into_owned()));
        let mut invoked = vec![self.binary.display().to_string()];
        invoked.extend(full.iter().map(|arg| (*arg).to_owned()));
        Ok(CliRun {
            argv: invoked,
            exit_code: output.status.code(),
            stdout,
        })
    }
}

impl HolderProcess {
    /// The internal holder-loss probe: SIGKILL to this holder alone.
    pub(super) fn kill(self) -> Result<(), SoakError> {
        self.started.kill().map(drop)
    }

    /// Awaits the exit `service stop` asked for.
    pub(super) fn reap(self) -> Result<Option<i32>, SoakError> {
        let status = self.started.wait((READY, "stopped holder exit"))?;
        Ok(status.code())
    }
}

/// Bytes of every file below `root`, from file metadata.
pub(super) fn disk_bytes(root: &Path) -> Result<u64, SoakError> {
    let mut total = 0_u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).map_err(SoakError::io("list the run root"))? {
            let entry = entry.map_err(SoakError::io("list the run root"))?;
            let kind = entry
                .file_type()
                .map_err(SoakError::io("stat the run root"))?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                let length = entry
                    .metadata()
                    .map_err(SoakError::io("stat the run root"))?
                    .len();
                total = total.saturating_add(length);
            }
        }
    }
    Ok(total)
}

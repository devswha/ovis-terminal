//! Git invoked by argv in an explicit directory: no shell, no inherited `GIT_*`
//! redirection, no prompt, a bounded wait and a kill when the operation is cancelled.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use super::{Cancel, WorktreeError};

/// The Git executable and the longest one command may run.
#[derive(Debug, Clone)]
pub struct Git {
    program: PathBuf,
    bound: Duration,
}

/// One command: what it is for, where it runs and its arguments after `-C <dir>`.
#[derive(Debug)]
pub(super) struct Invocation<'a> {
    pub operation: &'static str,
    pub dir: &'a Path,
    pub args: Vec<&'a OsStr>,
}

/// A finished command's status and standard output.
#[derive(Debug)]
pub(super) struct Ran {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
}

impl Git {
    /// Uses `program` (for example `git` resolved through `PATH`) with a per-command bound.
    #[must_use]
    pub const fn new(program: PathBuf, bound: Duration) -> Self {
        Self { program, bound }
    }

    /// Runs one command; a cancelled or overdue child is killed, never left running.
    pub(super) async fn run(
        &self,
        call: Invocation<'_>,
        cancel: &mut Cancel,
    ) -> Result<Ran, WorktreeError> {
        let mut command = tokio::process::Command::new(&self.program);
        command
            .arg("-C")
            .arg(call.dir)
            .args(&call.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C");
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("GIT_") && name != "GIT_TERMINAL_PROMPT" {
                command.env_remove(name);
            }
        }
        let child = command.spawn().map_err(|source| WorktreeError::Spawn {
            operation: call.operation,
            source,
        })?;
        tokio::select! {
            output = child.wait_with_output() => {
                let output = output.map_err(|source| WorktreeError::Spawn {
                    operation: call.operation,
                    source,
                })?;
                Ok(Ran {
                    success: output.status.success(),
                    code: output.status.code(),
                    stdout: output.stdout,
                })
            }
            () = cancel.requested() => Err(WorktreeError::Interrupted { committed: Vec::new() }),
            () = tokio::time::sleep(self.bound) => Err(WorktreeError::TimedOut {
                operation: call.operation,
                bound_ms: u64::try_from(self.bound.as_millis()).unwrap_or(u64::MAX),
            }),
        }
    }

    /// Runs one command that must succeed and returns its standard output.
    pub(super) async fn output(
        &self,
        call: Invocation<'_>,
        cancel: &mut Cancel,
    ) -> Result<Vec<u8>, WorktreeError> {
        let operation = call.operation;
        let ran = self.run(call, cancel).await?;
        if ran.success {
            Ok(ran.stdout)
        } else {
            Err(WorktreeError::Vcs {
                operation,
                code: ran.code,
            })
        }
    }
}

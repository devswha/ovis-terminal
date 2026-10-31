//! Launching a holder outside any service manager, in its own session.
//!
//! The holder leads a new session, so its launcher's exit, terminal hangup or SSH
//! disconnect does not end it. That is not logout survival: a login manager may still
//! end every process of the user, which is why status reports the missing manager.

use std::io::{BufRead as _, BufReader, Read as _};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use ovis_terminal_protocol::domain::unix_service::{BootProof, DiscoveryRecord};
use serde::Deserialize;

use super::{ServiceError, ServicePaths};
use crate::unix::spawn_detached;

const READY_BOUND: Duration = Duration::from_secs(10);

/// The one line a detached holder writes on stdout once it serves.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadyLine {
    ready: DiscoveryRecord,
}

/// The line `program service run --manager detached` prints when ready.
#[must_use]
pub fn ready_line(record: &DiscoveryRecord) -> String {
    format!("{}\n", serde_json::json!({ "ready": record }))
}

/// The typed refusal `ovis-terminal --json service run` writes on stderr when it exits
/// without serving; other error fields are ignored here.
#[derive(Debug, Deserialize)]
struct Refusal {
    error: RefusalError,
}

#[derive(Debug, Deserialize)]
struct RefusalError {
    kind: String,
    holder: Option<BootProof>,
}

/// The holder process exited before serving: its typed refusal, else its diagnostics.
fn exited(stderr: &str, code: Option<i32>) -> ServiceError {
    match serde_json::from_str::<Refusal>(stderr.trim()) {
        Ok(refusal) if refusal.error.kind == "already_running" => ServiceError::AlreadyRunning {
            holder: refusal.error.holder.map(Box::new),
        },
        _ => failed(stderr, code),
    }
}

fn failed(stderr: &str, code: Option<i32>) -> ServiceError {
    ServiceError::ManagerFailed {
        program: "ovis-terminal service run",
        code,
        stderr: stderr.trim().to_owned(),
    }
}

/// Starts `program service run` detached and waits, bounded, for its ready line.
///
/// # Errors
///
/// [`ServiceError::NotReady`] after killing the holder it started when no ready line
/// arrives in time; [`ServiceError::AlreadyRunning`] with the live holder's boot proof
/// when the started holder lost the start lock to another; otherwise
/// [`ServiceError::ManagerFailed`] with the holder's diagnostics when it exits first.
pub fn launch_detached(
    paths: &ServicePaths,
    program: &Path,
) -> Result<DiscoveryRecord, ServiceError> {
    let io = |operation| move |source| ServiceError::Io { operation, source };
    let mut command = Command::new(program);
    command
        .args([
            "--json",
            "service",
            "run",
            "--session",
            paths.session().as_str(),
            "--manager",
            "detached",
        ])
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (var, value) in paths.environment().vars() {
        command.env(var.name(), value);
    }
    let mut child = spawn_detached(&mut command).map_err(io("start the holder"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| failed("stdout was not piped", None))?;
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut line = String::new();
        let read = BufReader::new(stdout).read_line(&mut line);
        sender.send(read.map(|_| line)).is_ok()
    });
    let line = receiver.recv_timeout(READY_BOUND);
    let ready = line
        .as_ref()
        .ok()
        .and_then(|read| read.as_ref().ok())
        .and_then(|text| serde_json::from_str::<ReadyLine>(text).ok());
    if let Some(ready) = ready {
        reader
            .join()
            .map_err(|_| failed("the ready reader panicked", None))?;
        return Ok(ready.ready);
    }
    if line.is_err() {
        child.kill().map_err(io("end the unready holder"))?;
    }
    let status = child.wait().map_err(io("reap the holder"))?;
    reader
        .join()
        .map_err(|_| failed("the ready reader panicked", None))?;
    if line.is_err() {
        return Err(ServiceError::NotReady {
            bound_ms: u64::try_from(READY_BOUND.as_millis()).unwrap_or(u64::MAX),
        });
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_string(&mut stderr)
            .map_err(io("read the holder diagnostics"))?;
    }
    Err(exited(&stderr, status.code()))
}

#[cfg(test)]
mod tests {
    use super::{ServiceError, exited};

    #[test]
    fn a_losing_holder_refusal_becomes_typed_already_running_with_the_winner_proof() {
        let refusal = r#"{"error":{"exit_code":11,"hint":"h","kind":"already_running","message":"m","origin":"cli","holder":{"pid":4242,"start_token":"linux:b:7"}}}"#;
        let ServiceError::AlreadyRunning {
            holder: Some(holder),
        } = exited(refusal, Some(11))
        else {
            panic!("not a typed already_running");
        };
        assert_eq!(
            (holder.pid.get(), holder.start_token.as_str()),
            (4242, "linux:b:7")
        );
        let other = exited(r#"{"error":{"kind":"service_io"}}"#, Some(1));
        assert_eq!(other.kind(), "manager_failed");
        assert_eq!(exited("not json", None).kind(), "manager_failed");
    }
}

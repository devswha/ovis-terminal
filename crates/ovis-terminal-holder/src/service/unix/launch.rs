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

use ovis_terminal_protocol::domain::unix_service::DiscoveryRecord;
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
/// arrives in time, or [`ServiceError::ManagerFailed`] with the holder's diagnostics when
/// it exits first.
pub fn launch_detached(
    paths: &ServicePaths,
    program: &Path,
) -> Result<DiscoveryRecord, ServiceError> {
    let io = |operation| move |source| ServiceError::Io { operation, source };
    let mut command = Command::new(program);
    command
        .args([
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
    Err(failed(&stderr, status.code()))
}

//! One bounded run of the binary under test in the isolated root.

use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;

use super::transport::Endpoint;
use crate::error::RunnerError;
use crate::evidence::{CapturedText, ProcessReport};
use crate::scenario::model::Run;

const RUN_BOUND: Duration = Duration::from_secs(60);

/// What a finished run produced.
#[derive(Debug)]
pub(crate) struct Ran {
    pub(crate) exit: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) report: ProcessReport,
}

/// Runs `binary <args>`; with `run.endpoint` it also gets the fixture socket and credential.
pub(crate) async fn run(
    endpoint: &Endpoint,
    run: &Run,
    bearer: Option<&str>,
) -> Result<Ran, RunnerError> {
    let mut command = tokio::process::Command::new(&endpoint.binary);
    command
        .args(&run.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    endpoint.isolate(&mut command);
    if !run.endpoint {
        command.env_remove("OVIS_TERMINAL_SOCKET");
    }
    if let (true, Some(bearer)) = (run.endpoint, bearer) {
        command.env("OVIS_TERMINAL_CREDENTIAL", bearer);
    }
    let child = command.spawn().map_err(|source| RunnerError::Spawn {
        program: endpoint.binary.clone(),
        source,
    })?;
    let pid = child.id();
    let output = tokio::time::timeout(RUN_BOUND, child.wait_with_output())
        .await
        .map_err(|_| RunnerError::Timeout {
            waiting_for: "the binary to exit",
            bound_ms: u64::try_from(RUN_BOUND.as_millis()).unwrap_or(u64::MAX),
        })?
        .map_err(|source| RunnerError::Io {
            operation: "wait for the binary",
            source,
        })?;
    let mut argv = vec![endpoint.binary.display().to_string()];
    argv.extend(run.args.iter().cloned());
    let report = ProcessReport {
        role: "binary",
        argv,
        pid: pid.map(|pid| crate::ids::Decimal(u64::from(pid))),
        exit_code: output.status.code(),
        stdout: CapturedText::of(&output.stdout),
        stderr: CapturedText::of(&output.stderr),
    };
    Ok(Ran {
        exit: output.status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
        report,
    })
}

/// A captured stream parsed as one JSON document; non-JSON output is null.
pub(crate) fn document(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or(Value::Null)
}

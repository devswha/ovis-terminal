//! Readiness of a selected executable: it starts with a fixed argv and an empty
//! environment, answers `--json --version` with the expected version and a protocol
//! generation its manifest declares, and exits successfully before the deadline.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use ovis_terminal_protocol::domain::update::{ProbeOutcome, ReadinessEvidence, ReleaseVersion};
use ovis_terminal_protocol::schema::scalar::ProtocolGeneration;
use serde::Deserialize;
use tokio::io::AsyncReadExt as _;
use tokio::process::Command;
use tokio::runtime::Handle;

use super::UpdateError;

const STDOUT_LIMIT: u64 = 4096;
const EXCERPT_CHARS: usize = 1024;

/// What a ready build must report.
#[derive(Debug, Clone, Copy)]
pub(super) struct Expectation<'a> {
    pub(super) version: &'a ReleaseVersion,
    pub(super) protocols: &'a [ProtocolGeneration],
    pub(super) deadline: Duration,
}

#[derive(Deserialize)]
struct VersionReport {
    version: String,
    generation: ProtocolGeneration,
}

fn evidence(outcome: ProbeOutcome, code: Option<i32>, output: &[u8]) -> ReadinessEvidence {
    let stdout = String::from_utf8_lossy(output)
        .chars()
        .take(EXCERPT_CHARS)
        .collect();
    ReadinessEvidence::new(outcome, (code, stdout))
}

fn judge(output: &[u8], expect: &Expectation<'_>) -> ProbeOutcome {
    match serde_json::from_slice::<VersionReport>(output) {
        Ok(report)
            if report.version == expect.version.as_str()
                && expect.protocols.contains(&report.generation) =>
        {
            ProbeOutcome::Ready
        }
        Ok(_) | Err(_) => ProbeOutcome::Mismatched,
    }
}

async fn run(binary: &Path, expect: &Expectation<'_>) -> ReadinessEvidence {
    let spawned = Command::new(binary)
        .args(["--json", "--version"])
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn();
    let Ok(mut child) = spawned else {
        return evidence(ProbeOutcome::SpawnFailed, None, &[]);
    };
    let Some(stdout) = child.stdout.take() else {
        return evidence(ProbeOutcome::SpawnFailed, None, &[]);
    };
    let mut output = Vec::new();
    let finished = tokio::time::timeout(expect.deadline, async {
        stdout.take(STDOUT_LIMIT).read_to_end(&mut output).await?;
        child.wait().await
    })
    .await;
    match finished {
        Err(_elapsed) => {
            // The child is ours and still running; a failed kill means it already exited.
            match child.kill().await {
                Ok(()) | Err(_) => {}
            }
            evidence(ProbeOutcome::TimedOut, None, &output)
        }
        Ok(Err(_read)) => evidence(ProbeOutcome::Exited, None, &output),
        Ok(Ok(status)) if !status.success() => {
            evidence(ProbeOutcome::Exited, status.code(), &output)
        }
        Ok(Ok(status)) => evidence(judge(&output, expect), status.code(), &output),
    }
}

/// Probes `binary` on the current Tokio runtime from a blocking thread.
pub(super) fn probe(
    binary: &Path,
    expect: &Expectation<'_>,
) -> Result<ReadinessEvidence, UpdateError> {
    Ok(Handle::try_current()?.block_on(run(binary, expect)))
}

//! A program that never reads its input must stay killable while a writer is
//! blocked on its full input queue.
//!
//! The blocking writer runs in the stall fixture subprocess, never in the
//! caller: a writer blocked in the kernel cannot be cancelled from user space.
//! The probe bounds the fixture, SIGKILLs it, reaps it and joins its own report
//! reader before returning, so no blocked thread or descriptor outlives it.

use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;
use std::process::{ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::error::{ProbeError, PtyOperation};
use super::measure::{ByteCount, Micros};
use super::process::Pid;
#[cfg(unix)]
use super::process::{ProcessState, observe_process};
use super::session::ExitRecord;

/// What the blocked writer did after the program was killed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum WriterAfterKill {
    /// The writer finished; `error` names the OS error kind when it failed.
    Released {
        /// OS error kind of the failed write, if any.
        error: Option<String>,
    },
    /// The writer was still blocked one second after the program died.
    StillBlocked,
}

/// What the fixture measured.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StallReport {
    /// Input the writer attempted.
    pub attempted: ByteCount,
    /// Input accepted before the writer stopped making progress.
    pub accepted_before_kill: ByteCount,
    /// Whether the writer was blocked when the kill was sent.
    pub writer_blocked: bool,
    /// Whether the backend's own kill ended the program within one second.
    pub backend_kill_ended_program: bool,
    /// The backend kill's error, when it failed outright.
    pub backend_kill_error: Option<String>,
    /// The exit finally collected.
    pub exit: ExitRecord,
    /// From the first kill to the collected exit.
    pub kill_to_exit: Micros,
    /// What the writer did after the program died.
    pub writer: WriterAfterKill,
}

/// The measurement plus the containment the probe performed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct StallOutcome {
    /// The fixture's measurement.
    pub report: StallReport,
    /// Whether the writer was still blocked when the probe killed the fixture.
    pub writer_still_blocked_at_kill: bool,
    /// The fixture process, killed and reaped before the probe returned.
    pub fixture: Pid,
    /// The program that never read, verified gone before the probe returned.
    pub program: Pid,
}

enum FixtureLine {
    Program(Option<Pid>),
    Report(String),
}

/// Runs `fixture stall` under `bound` per step, then kills and reaps it.
///
/// # Errors
///
/// [`ProbeError::SpawnFailed`] when the fixture cannot start,
/// [`ProbeError::Timeout`] when it reports nothing in time,
/// [`ProbeError::Io`] when it exits without a report (its stderr becomes the
/// source) and [`ProbeError::MalformedReport`] for an unparsable report.
/// The fixture is killed and reaped on every path.
pub fn probe_stalled_writer(fixture: &Path, bound: Duration) -> Result<StallOutcome, ProbeError> {
    let mut child = Command::new(fixture)
        .arg("stall")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| ProbeError::SpawnFailed {
            source: error.into(),
        })?;
    let fixture_pid = Pid::new(child.id());
    let stdout = child.stdout.take().ok_or(ProbeError::OutputEnded)?;
    let (sender, lines) = sync_channel(2);
    let reader = thread::Builder::new()
        .name("probe-stall-report".into())
        .spawn(move || forward(stdout, &sender))
        .map_err(|source| ProbeError::Io {
            operation: PtyOperation::StartThread,
            source,
        })?;
    let waited = bound.saturating_mul(3);
    let (program, report) = collect(&lines, waited);
    let killed = child.kill().is_ok();
    let status = child.wait().map_err(|source| ProbeError::Io {
        operation: PtyOperation::Wait,
        source,
    })?;
    reader.join().map_err(|_panic| ProbeError::Io {
        operation: PtyOperation::StartThread,
        source: io::Error::other("the fixture report reader panicked"),
    })?;
    #[cfg(unix)]
    if let Some(pid) = program {
        reap_program(pid)?;
    }
    let report = match report {
        Ok(report) => report,
        Err(RecvTimeoutError::Timeout) => return Err(ProbeError::Timeout { waited }),
        Err(RecvTimeoutError::Disconnected) => {
            let stderr = child.stderr.take().map(read_all).unwrap_or_default();
            return Err(ProbeError::Io {
                operation: PtyOperation::Wait,
                source: io::Error::other(format!(
                    "the stall fixture exited ({status}) without a report: {}",
                    stderr.trim()
                )),
            });
        }
    };
    let report: StallReport =
        serde_json::from_str(&report).map_err(|_unparsable| ProbeError::MalformedReport {
            report: "STALL <StallReport json>",
        })?;
    Ok(StallOutcome {
        writer_still_blocked_at_kill: killed && report.writer == WriterAfterKill::StillBlocked,
        report,
        fixture: fixture_pid,
        program: program.ok_or(ProbeError::MalformedReport {
            report: "PROGRAM <pid>",
        })?,
    })
}

fn forward(stdout: ChildStdout, sender: &SyncSender<FixtureLine>) {
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let parsed = if let Some(pid) = line.strip_prefix("PROGRAM ") {
            FixtureLine::Program(pid.trim().parse().ok().map(Pid::new))
        } else if let Some(json) = line.strip_prefix("STALL ") {
            FixtureLine::Report(json.to_owned())
        } else {
            continue;
        };
        if sender.send(parsed).is_err() {
            return;
        }
    }
}

/// The program PID and the report line, or why the report is missing.
fn collect(
    lines: &Receiver<FixtureLine>,
    bound: Duration,
) -> (Option<Pid>, Result<String, RecvTimeoutError>) {
    let mut program = None;
    loop {
        match lines.recv_timeout(bound) {
            Ok(FixtureLine::Program(pid)) => program = pid,
            Ok(FixtureLine::Report(report)) => return (program, Ok(report)),
            Err(missing) => return (program, Err(missing)),
        }
    }
}

/// The program ignores SIGHUP, so a fixture that died early can leave it
/// running; it is killed here rather than left to the init process. On
/// Windows, closing the pseudoconsole with the fixture ends its programs.
#[cfg(unix)]
fn reap_program(pid: Pid) -> Result<(), ProbeError> {
    match observe_process(pid) {
        Ok(observed) if observed.state == ProcessState::Live => super::process::force_kill(pid),
        Ok(_) | Err(_) => Ok(()),
    }
}

fn read_all(mut pipe: impl Read) -> String {
    let mut text = String::new();
    match pipe.read_to_string(&mut text) {
        Ok(_) => text,
        Err(error) => format!("<stderr unreadable: {error}>"),
    }
}

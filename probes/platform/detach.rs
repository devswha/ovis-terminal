//! Holder survival: a holder started from a login terminal, in its own process
//! group, keeps its pseudo-terminal program after the login ends.
//!
//! The fixture's `launch` mode runs in a login terminal, starts its `serve`
//! mode in a new process group and relays `HELD holder=<pid> program=<pid>`.
//! `serve` owns a pseudo-terminal running `cat` until that program exits.

use std::path::Path;
use std::time::Duration;

use portable_pty::CommandBuilder;
use serde::Serialize;

use super::error::ProbeError;
use super::process::{Pid, ProcessObservation, force_kill, observe_process};
use super::session::{ExitRecord, Grid, PtySession};

/// How the login that started the holder ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Interruption {
    /// The launching process is killed with SIGKILL, as a crashing core is.
    CoreDeath,
    /// The launching process receives SIGHUP, as on SSH logout.
    Logout,
}

/// A process observed before and after the interruption.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct SurvivingProcess {
    /// Observation before the interruption.
    pub before: ProcessObservation,
    /// Observation after the launcher's exit was collected.
    pub after: ProcessObservation,
}

/// Outcome of one detach probe.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct DetachReport {
    /// The interruption applied.
    pub interruption: Interruption,
    /// Exit of the launching process.
    pub launcher_exit: ExitRecord,
    /// The holder process.
    pub holder: SurvivingProcess,
    /// The program the holder runs in its pseudo-terminal.
    pub program: SurvivingProcess,
}

/// Kills the probe's detached processes however the probe ends.
struct Reap(Vec<Pid>);

impl Drop for Reap {
    fn drop(&mut self) {
        for pid in &self.0 {
            if let Err(error) = force_kill(*pid) {
                tracing::warn!(%pid, %error, "a detached probe process could not be killed");
            }
        }
    }
}

/// Starts `fixture launch` in a login terminal, applies `interruption` to the
/// launcher, then observes the holder and its program again.
///
/// # Errors
///
/// Any session or process-record error; the detached processes are killed
/// before returning either way.
pub fn probe_detach(
    fixture: &Path,
    interruption: Interruption,
    bound: Duration,
) -> Result<DetachReport, ProbeError> {
    let mut command = CommandBuilder::new(fixture);
    command.arg("launch");
    let mut login = PtySession::spawn(command, Grid::new(80, 24))?;
    login.read_until(b"HELD ", bound)?;
    let line = login.read_until(b"\n", bound)?;
    let (holder_pid, program_pid) = parse_held(&line)?;
    let _reap = Reap(vec![program_pid, holder_pid]);
    let holder_before = observe_process(holder_pid)?;
    let program_before = observe_process(program_pid)?;
    match interruption {
        Interruption::CoreDeath => force_kill(login.pid().ok_or(ProbeError::OutputEnded)?)?,
        Interruption::Logout => login.kill()?,
    }
    let launcher_exit = login.wait_exit(bound)?;
    login.finish(bound)?;
    Ok(DetachReport {
        interruption,
        launcher_exit,
        holder: SurvivingProcess {
            before: holder_before,
            after: observe_process(holder_pid)?,
        },
        program: SurvivingProcess {
            before: program_before,
            after: observe_process(program_pid)?,
        },
    })
}

fn parse_held(line: &[u8]) -> Result<(Pid, Pid), ProbeError> {
    let malformed = || ProbeError::MalformedReport {
        report: "HELD holder=<pid> program=<pid>",
    };
    let text = std::str::from_utf8(line).map_err(|_not_utf8| malformed())?;
    let field = |name: &str| {
        text.split_whitespace()
            .find_map(|pair| pair.strip_prefix(name))
            .and_then(|digits| digits.parse().ok())
            .map(Pid::new)
            .ok_or_else(malformed)
    };
    Ok((field("holder=")?, field("program=")?))
}

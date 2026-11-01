//! Crash-recovery admission and evidence for the routes assembly composes (plan todo 49).
//!
//! After an unplanned boot the core's [`CrashRecovery`] report decides what work a
//! terminal may receive: a terminal whose survival is unknown refuses input and relaunch
//! with `recovering` (no duplicate spawn, no write into a program nobody proved), a
//! survivor refuses relaunch with `not_lost`, and only a proven loss or restored metadata
//! may be relaunched explicitly. `server.metrics` evidence and the incident journal are
//! read here and never cleared by a read. The served binary that registers these checks
//! in front of `input.*` and `terminal.recover` is assembled by plan todo 31.

use std::sync::Arc;

use ovis_terminal_core::recovery::{Incidents, RecoveryError};
use ovis_terminal_protocol::domain::recovery::{
    CrashRecovery, RecoveryIncident, Survival, TerminalRecovery,
};
use ovis_terminal_protocol::schema::ids::TerminalId;
use ovis_terminal_protocol::schema::methods::server::RecoveryEvidence;
use ovis_terminal_protocol::schema::scalar::TimestampMs;

use crate::dispatch::DispatchError;

#[cfg(test)]
mod tests;

/// Why recovery refuses work on a terminal.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RecoveryRefusal {
    /// Survival of the terminal's program is not settled.
    #[error("the terminal's program has unproven survival after an unplanned boot")]
    Recovering,
    /// The program was proven to survive or was already relaunched.
    #[error("the terminal's program is not lost")]
    NotLost,
    /// The recovery report does not name the terminal.
    #[error("the recovery report names no such terminal")]
    NotFound,
}

impl RecoveryRefusal {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Recovering => "recovering",
            Self::NotLost => "not_lost",
            Self::NotFound => "not_found",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Recovering => {
                "inspect the holder group and boot evidence; nothing is relaunched or written until survival is settled"
            }
            Self::NotLost => "the program still runs; attach to it instead of relaunching",
            Self::NotFound => "list terminals to find the current identity",
        }
    }
}

fn refused(refusal: RecoveryRefusal) -> DispatchError {
    DispatchError::Domain {
        kind: refusal.kind(),
        hint: refusal.hint(),
        source: Box::new(refusal),
    }
}

fn failed(error: RecoveryError) -> DispatchError {
    DispatchError::Domain {
        kind: error.kind(),
        hint: error.hint(),
        source: Box::new(error),
    }
}

/// The recovery report of the current boot and its incident journal.
#[derive(Debug)]
pub struct RecoveryLane {
    report: CrashRecovery,
    incidents: Incidents,
}

impl RecoveryLane {
    /// Serves `report`, reading incidents from `incidents`.
    #[must_use]
    pub fn new(report: CrashRecovery, incidents: Incidents) -> Arc<Self> {
        Arc::new(Self { report, incidents })
    }

    fn terminal(&self, id: &TerminalId) -> Option<&TerminalRecovery> {
        self.report
            .terminals
            .iter()
            .find(|terminal| terminal.terminal_id == *id)
    }

    /// Admits input to `id` unless its program's survival is unknown; a terminal the
    /// report does not name was created after recovery and is admitted.
    ///
    /// # Errors
    ///
    /// `recovering` for an unknown survival.
    pub fn admit_input(&self, id: &TerminalId) -> Result<(), DispatchError> {
        match self.terminal(id).map(|terminal| terminal.survival) {
            Some(Survival::Unknown { .. }) => Err(refused(RecoveryRefusal::Recovering)),
            _ => Ok(()),
        }
    }

    /// Admits an explicit relaunch of `id` only when its program is proven gone.
    ///
    /// # Errors
    ///
    /// `recovering` while survival is unknown, `not_lost` for a survivor or a program
    /// already relaunched, and `not_found` for a terminal the report does not name.
    pub fn admit_relaunch(&self, id: &TerminalId) -> Result<(), DispatchError> {
        let terminal = self
            .terminal(id)
            .ok_or_else(|| refused(RecoveryRefusal::NotFound))?;
        match terminal.survival {
            survival if survival.may_relaunch() => Ok(()),
            Survival::Unknown { .. } => Err(refused(RecoveryRefusal::Recovering)),
            _ => Err(refused(RecoveryRefusal::NotLost)),
        }
    }

    /// The `server.metrics` recovery evidence of this boot, recorded at `recorded_ms`.
    #[must_use]
    pub fn evidence(&self, recorded_ms: TimestampMs) -> RecoveryEvidence {
        self.report.evidence(recorded_ms)
    }

    /// The report this boot recovered with.
    #[must_use]
    pub const fn report(&self) -> &CrashRecovery {
        &self.report
    }

    /// Every durable incident; reading clears nothing.
    ///
    /// # Errors
    ///
    /// `journal_corrupt` or a persistence failure; the journal is never rewritten.
    pub fn incidents(&self) -> Result<Vec<RecoveryIncident>, DispatchError> {
        self.incidents.list().map_err(failed)
    }
}

use std::sync::{PoisonError, RwLock};

use ovis_terminal_protocol::domain::health::RecoveryState;
use ovis_terminal_protocol::domain::recovery::{
    CrashRecovery, IncidentKind, RecoveryIncident, Survival,
};
use ovis_terminal_protocol::schema::ids::ServerBootId;
use ovis_terminal_protocol::schema::methods::server::RecoveryEvidence;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_store::{RecoveryCause, RecoveryEvidence as Quarantined};

use super::HealthError;
use crate::recovery::{Finding, Incidents, RecoveryError};

const fn damaged(kind: IncidentKind) -> bool {
    matches!(
        kind,
        IncidentKind::StoreCorrupted | IncidentKind::CheckpointCorrupted
    )
}

/// What the journal held when it was last published.
#[derive(Debug, Clone, Copy)]
struct Journaled {
    incidents: u64,
    corrupted: bool,
}

/// The recovery facts of this boot: the crash-recovery report, if the boot recovered,
/// and the durable incident journal. Facts are only ever added; no read removes one.
#[derive(Debug)]
pub struct RecoveryLedger {
    journal: Incidents,
    boot: ServerBootId,
    report: Option<CrashRecovery>,
    recorded_ms: TimestampMs,
    journaled: RwLock<Journaled>,
}

impl RecoveryLedger {
    /// Reads `journal` once for boot `boot`, recovered at `recorded_ms` with `report`.
    /// A journal whose lines no longer decode is damaged material: it is reported as
    /// corrupted rather than refused, and it is never rewritten.
    ///
    /// # Errors
    ///
    /// [`HealthError::Journal`] when the journal exists but cannot be read at all.
    pub fn open(
        journal: Incidents,
        (boot, recorded_ms): (ServerBootId, TimestampMs),
        report: Option<CrashRecovery>,
    ) -> Result<Self, HealthError> {
        let journaled = match journal.list() {
            Ok(incidents) => Journaled {
                incidents: u64::try_from(incidents.len()).unwrap_or(u64::MAX),
                corrupted: incidents.iter().any(|incident| damaged(incident.kind)),
            },
            Err(RecoveryError::JournalCorrupt { .. }) => Journaled {
                incidents: 0,
                corrupted: true,
            },
            Err(source) => return Err(HealthError::Journal { source }),
        };
        Ok(Self {
            journal,
            boot,
            report,
            recorded_ms,
            journaled: RwLock::new(journaled),
        })
    }

    /// Makes `finding` durable, then publishes it.
    ///
    /// # Errors
    ///
    /// [`HealthError::Record`] when the journal cannot sync it; nothing is published.
    pub fn record(&self, finding: Finding) -> Result<RecoveryIncident, HealthError> {
        let incident = self
            .journal
            .record(&self.boot, finding)
            .map_err(|source| HealthError::Record { source })?;
        let mut journaled = self
            .journaled
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        journaled.incidents = journaled.incidents.saturating_add(1);
        journaled.corrupted |= damaged(incident.kind);
        drop(journaled);
        Ok(incident)
    }

    /// The summary of the published facts plus the store's quarantined material.
    #[must_use]
    pub fn state(&self, quarantined: &[Quarantined]) -> RecoveryState {
        let journaled = *self
            .journaled
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let store_damaged = quarantined.iter().any(|evidence| {
            matches!(
                evidence.cause,
                RecoveryCause::CorruptHeader | RecoveryCause::CorruptDatabase { .. }
            )
        });
        let quarantine_count = u64::try_from(quarantined.len()).unwrap_or(u64::MAX);
        let unsettled = self.report.as_ref().map_or(0, |report| {
            let unknown = report
                .terminals
                .iter()
                .filter(|terminal| matches!(terminal.survival, Survival::Unknown { .. }))
                .count();
            u64::try_from(unknown).unwrap_or(u64::MAX)
        });
        RecoveryState::derive(
            journaled.corrupted || store_damaged,
            (
                journaled.incidents.saturating_add(quarantine_count),
                unsettled,
            ),
            self.report.is_some(),
        )
    }

    /// The boot's recovery evidence; empty when this boot did not recover.
    #[must_use]
    pub fn evidence(&self) -> Vec<RecoveryEvidence> {
        self.report
            .iter()
            .map(|report| report.evidence(self.recorded_ms))
            .collect()
    }
}

//! The durable incident journal: append-only, synced per record, never cleared by a read.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_protocol::domain::recovery::{IncidentId, IncidentKind, RecoveryIncident};
use ovis_terminal_protocol::schema::ids::{ServerBootId, TerminalId};
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_store::StoreError;

use super::RecoveryError;

/// What one incident says, before it is given an identity and a time.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Finding {
    /// What happened.
    pub kind: IncidentKind,
    /// Terminals it affects.
    pub terminals: Vec<TerminalId>,
    /// Where damaged or unsupported material was kept.
    pub quarantine: Option<PathBuf>,
}

impl Finding {
    /// `kind` affecting `terminals`, with material kept at `quarantine`.
    #[must_use]
    pub const fn new(
        kind: IncidentKind,
        terminals: Vec<TerminalId>,
        quarantine: Option<PathBuf>,
    ) -> Self {
        Self {
            kind,
            terminals,
            quarantine,
        }
    }

    /// The incident a store refusal records, with the location the store kept its
    /// database and WAL material at; `None` for failures that keep no material.
    #[must_use]
    pub fn of_store(error: &StoreError) -> Option<Self> {
        let kind = match error {
            StoreError::Corrupted { .. } => IncidentKind::StoreCorrupted,
            StoreError::Unsupported { .. } => IncidentKind::StoreUnsupported,
            _ => return None,
        };
        let location = error.recovery_evidence()?.location.clone();
        Some(Self::new(kind, Vec::new(), Some(location)))
    }
}

/// The journal file of a state directory.
#[derive(Debug, Clone)]
pub struct Incidents {
    path: PathBuf,
}

impl Incidents {
    /// The journal kept under `dir`.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join("incidents.jsonl"),
        }
    }

    /// Appends `finding` as an incident of `boot` and syncs it before returning.
    ///
    /// # Errors
    ///
    /// [`RecoveryError::JournalIo`] when the record cannot be made durable.
    pub fn record(
        &self,
        boot: &ServerBootId,
        finding: Finding,
    ) -> Result<RecoveryIncident, RecoveryError> {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
            .ok_or(RecoveryError::Clock)?;
        let incident = RecoveryIncident::new(
            (
                IncidentId::new(uuid::Uuid::now_v7().to_string()),
                finding.kind,
            ),
            (boot.clone(), TimestampMs::new(millis)),
            (
                finding.terminals,
                finding.quarantine.map(|path| path.display().to_string()),
            ),
        );
        let io = |operation| move |source| RecoveryError::JournalIo { operation, source };
        let mut line = serde_json::to_vec(&incident)
            .map_err(std::io::Error::other)
            .map_err(io("encoded"))?;
        line.push(b'\n');
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(io("created"))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(io("opened"))?;
        file.write_all(&line).map_err(io("appended"))?;
        file.sync_all().map_err(io("synced"))?;
        Ok(incident)
    }

    /// Every recorded incident, oldest first. Reading changes nothing.
    ///
    /// # Errors
    ///
    /// [`RecoveryError::JournalCorrupt`] naming the first line that is not an incident;
    /// the file is left exactly as it is.
    pub fn list(&self) -> Result<Vec<RecoveryIncident>, RecoveryError> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(RecoveryError::JournalIo {
                    operation: "read",
                    source,
                });
            }
        };
        text.lines()
            .enumerate()
            .map(|(index, line)| {
                serde_json::from_str(line).map_err(|source| RecoveryError::JournalCorrupt {
                    line: index.saturating_add(1),
                    source,
                })
            })
            .collect()
    }
}

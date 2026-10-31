//! The persisted status row of one terminal, read and written inside one commit unit.

use ovis_terminal_core::events::EventError;
use ovis_terminal_core::registry::{RuntimeId, TerminalId};
use ovis_terminal_protocol::schema::records::{AgentBinding, StatusSnapshot};
use ovis_terminal_protocol::schema::scalar::{EventSeq, Incarnation};
use ovis_terminal_store::{CasWrite, Payload, ResourceKey, ResourceLookup, Snapshot, Version};
use serde::{Deserialize, Serialize};

use super::StatusError;

/// Status, agent binding and turn bookkeeping of one terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Row {
    pub status: StatusSnapshot,
    pub agent: Option<AgentBinding>,
    /// Status event that started the current turn.
    pub started_seq: Option<EventSeq>,
    /// Latest status event of this terminal.
    pub last_event: EventSeq,
}

impl Row {
    pub(super) const fn unreported(process: Option<Incarnation>) -> Self {
        let mut status = StatusSnapshot::unreported(Incarnation::new(0));
        status.process_incarnation = process;
        Self {
            status,
            agent: None,
            started_seq: None,
            last_event: EventSeq::new(0),
        }
    }

    /// A new program starts unreported and unregistered; turn facts carry over.
    fn replaced(self, process: Option<Incarnation>) -> Self {
        let mut next = Self::unreported(process);
        next.status.turn = self.status.turn;
        next.status.finish_generation = self.status.finish_generation;
        next.status.last_turn = self.status.last_turn;
        next.last_event = self.last_event;
        next
    }
}

/// A row and the version its compare-and-swap write must expect.
#[derive(Debug)]
pub(super) struct Stored {
    pub row: Row,
    pub version: Version,
    pub terminal: TerminalId,
    key: ResourceKey,
}

/// Where a terminal's row lives.
#[derive(Debug, Clone, Copy)]
pub(super) struct Location {
    pub runtime: RuntimeId,
    pub terminal: TerminalId,
}

impl Location {
    fn key(self) -> ResourceKey {
        ResourceKey::new(
            "agent_status",
            format!("{}/{}", self.runtime, self.terminal),
        )
    }

    /// Reads the row as seen by program `process`; a row of another incarnation is replaced.
    pub(super) fn read(
        self,
        snapshot: &Snapshot,
        process: Option<Incarnation>,
    ) -> Result<Stored, StatusError> {
        let key = self.key();
        let (row, version) = match snapshot.resource(&key).map_err(EventError::from)? {
            ResourceLookup::Live { value, version, .. } => {
                (serde_json::from_slice::<Row>(value.as_bytes())?, version)
            }
            ResourceLookup::Absent => (Row::unreported(process), Version::ABSENT),
            _ => return Err(EventError::SnapshotUnavailable.into()),
        };
        let row = if row.status.process_incarnation == process {
            row
        } else {
            row.replaced(process)
        };
        Ok(Stored {
            row,
            version,
            terminal: self.terminal,
            key,
        })
    }
}

impl Stored {
    /// The compare-and-swap write of the updated row.
    pub(super) fn write(&self) -> Result<CasWrite, StatusError> {
        let bytes = serde_json::to_vec(&self.row)?;
        Ok(CasWrite::new(
            self.key.clone(),
            self.version,
            Some(Payload::from(bytes)),
        ))
    }
}

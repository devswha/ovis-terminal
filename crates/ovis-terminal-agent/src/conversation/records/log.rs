//! The persisted records of one agent, read and written inside one commit unit.

use std::collections::BTreeMap;

use ovis_terminal_core::events::EventError;
use ovis_terminal_core::registry::RuntimeId;
use ovis_terminal_protocol::schema::ids::{AgentId, HistoryId};
use ovis_terminal_protocol::schema::methods::agent::ConversationRecord;
use ovis_terminal_protocol::schema::scalar::{HistoryRevision, TurnNumber};
use ovis_terminal_store::{CasWrite, Payload, ResourceKey, ResourceLookup, Snapshot, Version};
use serde::{Deserialize, Serialize};

use super::RecordsError;
use super::turns::TurnFacts;

/// Most records one agent keeps.
pub(super) const RECORD_LIMIT: usize = 10_000;
/// Largest encoded record log, in bytes.
const BYTE_LIMIT: usize = 8 * 1024 * 1024;

/// Every record an agent reported, in append order, and the turns they belong to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RecordLog {
    pub history_id: HistoryId,
    /// Bumps once per accepted append.
    pub revision: HistoryRevision,
    pub records: Vec<ConversationRecord>,
    pub turns: BTreeMap<TurnNumber, TurnFacts>,
}

impl RecordLog {
    /// An empty log whose first append becomes revision 1.
    pub(super) fn empty() -> Self {
        Self {
            history_id: HistoryId::new(format!("records:{}", uuid::Uuid::now_v7())),
            revision: HistoryRevision::new(0),
            records: Vec::new(),
            turns: BTreeMap::new(),
        }
    }

    /// Facts of `turn`, starting empty.
    pub(super) fn turn(&mut self, turn: TurnNumber) -> &mut TurnFacts {
        self.turns.entry(turn).or_default()
    }
}

/// An agent's log, absent before its first append, and its compare-and-swap version.
#[derive(Debug)]
pub(super) struct StoredLog {
    pub log: Option<RecordLog>,
    version: Version,
    key: ResourceKey,
}

impl StoredLog {
    /// Reads the log of `agent` in `runtime`.
    pub(super) fn read(
        snapshot: &Snapshot,
        runtime: RuntimeId,
        agent: &AgentId,
    ) -> Result<Self, RecordsError> {
        let key = ResourceKey::new(
            "conversation_records",
            format!("{runtime}/{}", agent.as_str()),
        );
        let (log, version) = match snapshot.resource(&key).map_err(EventError::from)? {
            ResourceLookup::Live { value, version, .. } => {
                (Some(serde_json::from_slice(value.as_bytes())?), version)
            }
            ResourceLookup::Absent => (None, Version::ABSENT),
            _ => return Err(EventError::SnapshotUnavailable.into()),
        };
        Ok(Self { log, version, key })
    }

    /// The compare-and-swap write of `log`, refused beyond the byte bound.
    pub(super) fn write(&self, log: &RecordLog) -> Result<CasWrite, RecordsError> {
        let bytes = serde_json::to_vec(log)?;
        if bytes.len() > BYTE_LIMIT {
            return Err(RecordsError::TooLarge);
        }
        Ok(CasWrite::new(
            self.key.clone(),
            self.version,
            Some(Payload::from(bytes)),
        ))
    }
}

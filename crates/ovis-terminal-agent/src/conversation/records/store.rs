//! The persisted records of one agent: a small head, one entry per record, and index
//! entries for record identities, tool calls and turns.
//!
//! No entry grows with the history, so every one stays far below the store's value bound;
//! the agent's own bounds are enforced from the head's counters.

use ovis_terminal_core::events::EventError;
use ovis_terminal_core::registry::RuntimeId;
use ovis_terminal_protocol::schema::ids::{AgentId, CallId, HistoryId, RecordId};
use ovis_terminal_protocol::schema::scalar::{HistoryRevision, TurnNumber};
use ovis_terminal_store::{CasWrite, Payload, ResourceKey, ResourceLookup, Snapshot, Version};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::RecordsError;

/// Most records one agent keeps.
pub(super) const RECORD_LIMIT: u64 = 10_000;
/// Most stored record bytes one agent keeps.
pub(super) const BYTE_LIMIT: u64 = 8 * 1024 * 1024;

/// Identity, revision and counters of an agent's records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Head {
    pub history_id: HistoryId,
    /// Bumps once per accepted append.
    pub revision: HistoryRevision,
    /// Records held; the next record takes this position.
    pub count: u64,
    /// Stored bytes of every record held.
    pub bytes: u64,
    /// The newest record, which the next one continues.
    pub last: Option<RecordId>,
}

impl Head {
    /// Records not reported yet; the first append makes revision 1.
    pub(super) fn empty() -> Self {
        Self {
            history_id: HistoryId::new(format!("records:{}", uuid::Uuid::now_v7())),
            revision: HistoryRevision::new(0),
            count: 0,
            bytes: 0,
            last: None,
        }
    }
}

/// Keys of one agent's entries.
#[derive(Debug, Clone)]
pub(super) struct Keys(String);

impl Keys {
    pub(super) fn new(runtime: RuntimeId, agent: &AgentId) -> Self {
        Self(format!("{runtime}/{}", agent.as_str()))
    }

    pub(super) fn head(&self) -> ResourceKey {
        ResourceKey::new("conversation_records", self.0.clone())
    }

    pub(super) fn record(&self, position: u64) -> ResourceKey {
        ResourceKey::new("conversation_record", format!("{}/{position}", self.0))
    }

    pub(super) fn id(&self, record: &RecordId) -> ResourceKey {
        ResourceKey::new(
            "conversation_record_id",
            format!("{}/{}", self.0, record.as_str()),
        )
    }

    pub(super) fn call(&self, call: &CallId) -> ResourceKey {
        ResourceKey::new("conversation_call", format!("{}/{}", self.0, call.as_str()))
    }

    pub(super) fn turn(&self, turn: TurnNumber) -> ResourceKey {
        ResourceKey::new("conversation_turn", format!("{}/{}", self.0, turn.get()))
    }
}

/// One stored value, absent until written, and the version its write must expect.
#[derive(Debug)]
pub(super) struct Entry<T> {
    pub value: Option<T>,
    version: Version,
    key: ResourceKey,
}

impl<T: Serialize + DeserializeOwned> Entry<T> {
    pub(super) fn read(snapshot: &Snapshot, key: ResourceKey) -> Result<Self, RecordsError> {
        let (value, version) = match snapshot.resource(&key).map_err(EventError::from)? {
            ResourceLookup::Live { value, version, .. } => {
                (Some(serde_json::from_slice(value.as_bytes())?), version)
            }
            ResourceLookup::Absent => (None, Version::ABSENT),
            _ => return Err(EventError::SnapshotUnavailable.into()),
        };
        Ok(Self {
            value,
            version,
            key,
        })
    }

    /// The compare-and-swap write of `value` over what was read.
    pub(super) fn write(&self, value: &T) -> Result<CasWrite, RecordsError> {
        Ok(put(
            self.key.clone(),
            self.version,
            &serde_json::to_vec(value)?,
        ))
    }
}

/// The write of `bytes` at `key`, expected at `version`.
pub(super) fn put(key: ResourceKey, version: Version, bytes: &[u8]) -> CasWrite {
    CasWrite::new(key, version, Some(Payload::from(bytes.to_vec())))
}

/// The write of a new entry that must not exist yet.
pub(super) fn create<T: Serialize>(key: ResourceKey, value: &T) -> Result<CasWrite, RecordsError> {
    Ok(put(key, Version::ABSENT, &serde_json::to_vec(value)?))
}

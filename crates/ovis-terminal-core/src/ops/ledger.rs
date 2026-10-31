//! Bounded operation index and revision-pinned operation rows.

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::server::OpRecord;
use ovis_terminal_protocol::schema::scalar::{EventSeq, RecordVersion, TimestampMs};
use ovis_terminal_store::{ResourceKey, ResourceLookup, Snapshot};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::params::Params;
use crate::events::{EventData, EventError, Filter, SnapshotProducer};
use crate::registry::{RuntimeId, ServerBootId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct OperationKey(String);

impl OperationKey {
    pub(super) fn new(principal: &PrincipalId, op: &OpId) -> Result<Self, serde_json::Error> {
        let bytes = serde_json::to_vec(&(principal, op))?;
        let digest = Sha256::digest(bytes);
        Ok(Self(format!("{digest:x}")))
    }

    pub(super) fn resource(&self, runtime: RuntimeId) -> ResourceKey {
        ResourceKey::new("operation", format!("{runtime}/{}", self.0))
    }

    /// The inspection copy of this operation's canonical params.
    pub(super) fn params_resource(&self, runtime: RuntimeId) -> ResourceKey {
        ResourceKey::new("operation_params", format!("{runtime}/{}", self.0))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Index {
    pub floor: TimestampMs,
    pub keys: Vec<OperationKey>,
}

impl Default for Index {
    fn default() -> Self {
        Self {
            floor: TimestampMs::new(0),
            keys: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Entry {
    pub principal: PrincipalId,
    #[serde(flatten)]
    pub params: Params,
    pub accepted_at: TimestampMs,
    pub boot: ServerBootId,
    pub record: OpRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Row {
    pub key: OperationKey,
    pub version: RecordVersion,
    pub entry: Entry,
}

/// A bounded, typed operation view captured through one WAL snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OperationSnapshot {
    pub(super) index: Index,
    pub(super) version: RecordVersion,
    pub(super) rows: Vec<Row>,
}

impl AsRef<Self> for OperationSnapshot {
    fn as_ref(&self) -> &Self {
        self
    }
}

/// Captures operations for one runtime; assembly may compose this producer's section.
#[derive(Debug)]
pub struct OperationProducer {
    runtime: RuntimeId,
}

impl OperationProducer {
    /// Binds inspection to the same runtime that owns the coordinator.
    pub const fn new(runtime: RuntimeId) -> Self {
        Self { runtime }
    }

    /// Reads indexed receipts from the supplied committed view.
    ///
    /// # Errors
    ///
    /// Returns a typed store/decode failure rather than an empty successful ledger.
    pub fn capture(&self, snapshot: &Snapshot) -> Result<OperationSnapshot, EventError> {
        let (index, version) = match snapshot.resource(&index_key(self.runtime))? {
            ResourceLookup::Live { value, version, .. } => (
                serde_json::from_slice::<Index>(value.as_bytes())?,
                RecordVersion::new(version.get()),
            ),
            ResourceLookup::Absent => (Index::default(), RecordVersion::new(0)),
            _ => return Err(EventError::SnapshotUnavailable),
        };
        if index.keys.len() > 1024 {
            return Err(EventError::LimitExceeded);
        }
        let mut rows = Vec::with_capacity(index.keys.len());
        for key in &index.keys {
            let ResourceLookup::Live {
                value,
                version,
                revision,
            } = snapshot.resource(&key.resource(self.runtime))?
            else {
                return Err(EventError::SnapshotUnavailable);
            };
            let mut entry: Entry = serde_json::from_slice(value.as_bytes())?;
            if OperationKey::new(&entry.principal, &entry.record.op_id)? != *key {
                return Err(EventError::InvalidRecord);
            }
            entry.record.updated_seq = EventSeq::new(revision.get());
            rows.push(Row {
                key: key.clone(),
                version: RecordVersion::new(version.get()),
                entry,
            });
        }
        Ok(OperationSnapshot {
            index,
            version,
            rows,
        })
    }
}

impl SnapshotProducer for OperationProducer {
    type Sections = OperationSnapshot;

    fn capture(&self, snapshot: &Snapshot, _filter: &Filter) -> Result<Self::Sections, EventError> {
        self.capture(snapshot)
    }
}

pub(super) fn index_key(runtime: RuntimeId) -> ResourceKey {
    ResourceKey::new("operation_index", runtime.to_string())
}

/// A factual operation transition, committed before subscribers can see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OperationEvent {
    /// Resulting operation record; its sequence is stamped under the commit barrier.
    #[serde(flatten)]
    pub operation: OpRecord,
}

impl OperationEvent {
    pub(super) fn from_record(record: &OpRecord) -> Self {
        Self {
            operation: record.clone(),
        }
    }
}

impl EventData for OperationEvent {
    fn event_type(&self) -> &'static str {
        use ovis_terminal_protocol::schema::methods::server::OpState;
        use ovis_terminal_protocol::schema::scalar::OpenEnum;
        match &self.operation.state {
            OpenEnum::Known(OpState::Accepted) => "op.accepted",
            OpenEnum::Known(OpState::Running) => "op.running",
            OpenEnum::Known(OpState::Committed) => "op.committed",
            OpenEnum::Known(OpState::Failed) => "op.failed",
            OpenEnum::Known(OpState::Cancelled) => "op.cancelled",
            OpenEnum::Known(_) | OpenEnum::Unknown(_) => "op.outcome_unknown",
        }
    }

    fn at_sequence(&mut self, seq: EventSeq) {
        self.operation.updated_seq = seq;
    }
}

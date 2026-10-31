//! Terminal registry capture through the coordinator's pinned WAL snapshot.

use ovis_terminal_protocol::schema::scalar::{EventSeq, RecordVersion, Revision};
use ovis_terminal_store::{ResourceKey, ResourceLookup, Snapshot};
use serde::{Deserialize, Serialize};

use super::Record;
use crate::events::{EventData, EventError, Filter, SnapshotProducer};
use crate::ops::{OperationEvent, OperationProducer, OperationSnapshot};
use crate::registry::{ProcessIdentity, RuntimeId, TerminalId};

/// Terminal and operation sections captured at the same durable revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TerminalSnapshot {
    /// Inspectable operation ledger.
    pub operations: OperationSnapshot,
    /// All retained terminal records.
    pub terminals: Vec<Record>,
}

impl AsRef<OperationSnapshot> for TerminalSnapshot {
    fn as_ref(&self) -> &OperationSnapshot {
        &self.operations
    }
}

/// Captures the terminal registry and operation ledger together.
#[derive(Debug)]
pub struct Producer {
    runtime: RuntimeId,
}

impl Producer {
    /// Binds the producer to a registry runtime lifetime.
    pub const fn new(runtime: RuntimeId) -> Self {
        Self { runtime }
    }
}

pub(super) fn index_key(runtime: RuntimeId) -> ResourceKey {
    ResourceKey::new("terminal_index", runtime.to_string())
}

pub(super) fn record_key(id: TerminalId) -> ResourceKey {
    ResourceKey::new("terminal", id.to_string())
}

pub(super) fn read_record(
    snapshot: &Snapshot,
    id: TerminalId,
) -> Result<(Record, RecordVersion), EventError> {
    let ResourceLookup::Live {
        value,
        revision,
        version,
    } = snapshot.resource(&record_key(id))?
    else {
        return Err(EventError::SnapshotUnavailable);
    };
    let mut record: Record = serde_json::from_slice(value.as_bytes())?;
    if record.id() != id {
        return Err(EventError::InvalidRecord);
    }
    if record
        .holder_target
        .as_ref()
        .is_some_and(|target| target.terminal_id.as_str() != id.to_string())
        || record.exit.as_ref().is_some_and(|exit| {
            record.holder_target.as_ref() != Some(&exit.target)
                || record.output_offset.get() != exit.final_offset
        })
        || (record.agent_owner.is_some()
            && (record.lifecycle.process().is_none() || record.hangup.is_some()))
    {
        return Err(EventError::InvalidRecord);
    }
    record.revision = Revision::new(revision.get());
    Ok((record, RecordVersion::new(version.get())))
}

pub(super) fn read_index(
    snapshot: &Snapshot,
    runtime: RuntimeId,
) -> Result<(Vec<TerminalId>, RecordVersion), EventError> {
    match snapshot.resource(&index_key(runtime))? {
        ResourceLookup::Live { value, version, .. } => {
            let ids: Vec<TerminalId> = serde_json::from_slice(value.as_bytes())?;
            if ids.len() > 1024 {
                return Err(EventError::LimitExceeded);
            }
            Ok((ids, RecordVersion::new(version.get())))
        }
        ResourceLookup::Absent => Ok((Vec::new(), RecordVersion::new(0))),
        _ => Err(EventError::SnapshotUnavailable),
    }
}

impl SnapshotProducer for Producer {
    type Sections = TerminalSnapshot;

    fn capture(&self, snapshot: &Snapshot, _filter: &Filter) -> Result<Self::Sections, EventError> {
        let (ids, _) = read_index(snapshot, self.runtime)?;
        let mut terminals = Vec::with_capacity(ids.len());
        for id in ids {
            terminals.push(read_record(snapshot, id)?.0);
        }
        Ok(TerminalSnapshot {
            operations: OperationProducer::new(self.runtime).capture(snapshot)?,
            terminals,
        })
    }
}

/// Lifecycle events and operation outcomes share the coordinator sequence domain.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum LifecycleEvent {
    /// Explicit control/program administrative scope.
    Control(super::ControlReceipt),
    /// Terminal identity and metadata admitted before any process effect.
    Admitted(Record),
    /// The holder spawn is outstanding.
    Pending(Record),
    /// PID-plus-start-token evidence established the process.
    Spawned(Record),
    /// PTY EOF observed, without a reaped-exit claim.
    Hangup(Record),
    /// Final output and process status retained.
    Exited(Record),
    /// No process was spawned.
    SpawnFailed(Record),
    /// Metadata changed without input or signals.
    Updated(Record),
    /// Metadata was tombstoned without terminating another process.
    Removed {
        /// Stable identity that was removed.
        terminal: TerminalId,
        /// Historical process affected by metadata removal, if one existed.
        process: Option<ProcessIdentity>,
    },
    /// Retains the released operation event's unwrapped `OpRecord` representation.
    #[serde(untagged)]
    Operation(OperationEvent),
}

impl From<OperationEvent> for LifecycleEvent {
    fn from(event: OperationEvent) -> Self {
        Self::Operation(event)
    }
}

impl EventData for LifecycleEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Control(receipt) => match receipt {
                super::ControlReceipt::Stopped { .. } => "server.control_stopped",
                super::ControlReceipt::HandoverPrepared { .. } => "server.handover_prepared",
                super::ControlReceipt::ProgramStop { .. } => "server.program_stop",
            },
            Self::Operation(event) => event.event_type(),
            Self::Admitted(_) => "terminal.created",
            Self::Pending(_) => "terminal.spawn_pending",
            Self::Spawned(_) => "terminal.spawned",
            Self::Hangup(_) => "terminal.hangup",
            Self::Exited(_) => "terminal.finished",
            Self::SpawnFailed(_) => "terminal.spawn_failed",
            Self::Updated(_) => "terminal.updated",
            Self::Removed { .. } => "terminal.removed",
        }
    }

    fn at_sequence(&mut self, seq: EventSeq) {
        match self {
            Self::Operation(event) => event.at_sequence(seq),
            Self::Admitted(record)
            | Self::Pending(record)
            | Self::Spawned(record)
            | Self::Hangup(record)
            | Self::Exited(record)
            | Self::SpawnFailed(record)
            | Self::Updated(record) => record.revision = Revision::new(seq.get()),
            Self::Control(_) | Self::Removed { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use ovis_terminal_protocol::errors::OpId;
    use ovis_terminal_protocol::schema::ids::MethodName;
    use ovis_terminal_protocol::schema::methods::server::OpRecord;

    use super::*;

    #[test]
    fn lifecycle_union_preserves_the_existing_operation_event_wire_shape() {
        // Given: a released operation event with an exact maximum sequence.
        let mut operation = OpRecord::accepted(
            OpId::new("000005f5-e100-7000-8000-000000000001".into()),
            MethodName::new("terminal.run".into()),
        );
        operation.updated_seq = EventSeq::new(u64::MAX);
        let original = OperationEvent { operation };
        let bytes = serde_json::to_vec(&original).expect("released event");

        // When: the terminal lifecycle union reads and re-encodes the old record.
        let decoded: LifecycleEvent =
            serde_json::from_slice(&bytes).expect("old event is accepted");
        let encoded = serde_json::to_value(&decoded).expect("union event");

        // Then: no envelope wrapper or numeric counter has changed the OpRecord contract.
        assert_eq!(
            encoded,
            serde_json::to_value(&original).expect("released value")
        );
        assert!(matches!(decoded, LifecycleEvent::Operation(event) if event == original));
    }
}

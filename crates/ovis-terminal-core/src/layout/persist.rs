//! One durable layout row per runtime, captured through the pinned WAL view.

use ovis_terminal_protocol::schema::scalar::Revision;
use ovis_terminal_store::{CasWrite, Payload, ResourceKey, ResourceLookup, Snapshot, Version};
use serde::{Deserialize, Serialize};

use super::model::{Layout, MAX_CLIENTS, MAX_PANES, MAX_TABS, MAX_WORKSPACES};
use crate::events::{EventError, Filter, Section, SnapshotProducer, SnapshotSections};
use crate::ops::{OperationProducer, OperationSnapshot};
use crate::registry::RuntimeId;
use crate::terminals::{self, Record};

fn layout_key(runtime: RuntimeId) -> ResourceKey {
    ResourceKey::new("layout", runtime.to_string())
}

/// Reads the layout row and its CAS version; an absent row is the empty layout.
pub(super) fn read_layout(
    snapshot: &Snapshot,
    runtime: RuntimeId,
) -> Result<(Layout, Version), EventError> {
    match snapshot.resource(&layout_key(runtime))? {
        ResourceLookup::Absent => Ok((Layout::empty(), Version::ABSENT)),
        ResourceLookup::Live {
            value,
            revision,
            version,
        } => {
            let mut layout: Layout = serde_json::from_slice(value.as_bytes())?;
            if layout.workspaces.len() > MAX_WORKSPACES || layout.selections.len() > MAX_CLIENTS {
                return Err(EventError::LimitExceeded);
            }
            for workspace in &layout.workspaces {
                if workspace.tabs.len() > MAX_TABS {
                    return Err(EventError::LimitExceeded);
                }
                for tab in &workspace.tabs {
                    if tab.panes.len() > MAX_PANES {
                        return Err(EventError::LimitExceeded);
                    }
                    tab.validate()?;
                }
            }
            layout.revision = Revision::new(revision.get());
            Ok((layout, version))
        }
        _ => Err(EventError::SnapshotUnavailable),
    }
}

pub(super) fn write_layout(
    runtime: RuntimeId,
    version: Version,
    layout: &Layout,
) -> Result<CasWrite, serde_json::Error> {
    Ok(CasWrite::new(
        layout_key(runtime),
        version,
        Some(Payload::from(serde_json::to_vec(layout)?)),
    ))
}

/// Layout and operation sections captured at one durable revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LayoutSnapshot {
    /// Inspectable operation ledger.
    pub operations: OperationSnapshot,
    /// Complete layout and selections.
    pub layout: Layout,
}

impl AsRef<OperationSnapshot> for LayoutSnapshot {
    fn as_ref(&self) -> &OperationSnapshot {
        &self.operations
    }
}

impl AsRef<Layout> for LayoutSnapshot {
    fn as_ref(&self) -> &Layout {
        &self.layout
    }
}

/// Captures the layout row and operation ledger together.
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

impl SnapshotProducer for Producer {
    type Sections = LayoutSnapshot;

    fn capture(&self, snapshot: &Snapshot, _filter: &Filter) -> Result<Self::Sections, EventError> {
        Ok(LayoutSnapshot {
            operations: OperationProducer::new(self.runtime).capture(snapshot)?,
            layout: read_layout(snapshot, self.runtime)?.0,
        })
    }
}

/// Session sections: layout and terminal records from the same read transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SessionSnapshot {
    /// Inspectable operation ledger.
    pub operations: OperationSnapshot,
    /// Layout and terminal sections at the snapshot cursor.
    pub sections: SnapshotSections<Layout, Vec<Record>>,
}

impl AsRef<OperationSnapshot> for SessionSnapshot {
    fn as_ref(&self) -> &OperationSnapshot {
        &self.operations
    }
}

/// Composes the layout and terminal producers into one atomic session snapshot.
#[derive(Debug)]
pub struct SessionProducer {
    runtime: RuntimeId,
    terminals: terminals::Producer,
}

impl SessionProducer {
    /// Binds both producers to one registry runtime lifetime.
    pub const fn new(runtime: RuntimeId) -> Self {
        Self {
            runtime,
            terminals: terminals::Producer::new(runtime),
        }
    }
}

impl SnapshotProducer for SessionProducer {
    type Sections = SessionSnapshot;

    fn capture(&self, snapshot: &Snapshot, filter: &Filter) -> Result<Self::Sections, EventError> {
        let captured = self.terminals.capture(snapshot, filter)?;
        Ok(SessionSnapshot {
            operations: captured.operations,
            sections: SnapshotSections::new(
                Section::Complete(read_layout(snapshot, self.runtime)?.0),
                Section::Complete(captured.terminals),
            ),
        })
    }
}

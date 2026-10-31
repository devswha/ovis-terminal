//! The persisted record of one history: its immutable source, the latest observation,
//! its revision and the chain head an owning process selected.

use ovis_terminal_core::events::EventError;
use ovis_terminal_core::registry::RuntimeId;
use ovis_terminal_protocol::domain::session_binding::{HistoryChange, HistoryHead};
use ovis_terminal_protocol::schema::ids::{HistoryId, RecordId};
use ovis_terminal_protocol::schema::scalar::HistoryRevision;
use ovis_terminal_store::{CasWrite, Payload, ResourceKey, ResourceLookup, Snapshot, Version};
use serde::{Deserialize, Serialize};

use super::observe::{Content, Observation};
use super::{BindingError, SourceRef};

/// What the runtime knows about one history identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct HistoryRow {
    /// The source this identity was first bound to; it never changes.
    pub source: SourceRef,
    /// Latest observation of a transcript source.
    pub observation: Option<Observation>,
    pub revision: HistoryRevision,
    /// What moved the history to `revision`.
    pub last_change: Option<HistoryChange>,
    /// Chain head selected by the owning process.
    pub selected: Option<Selection>,
}

/// A chain head chosen by the owning process, and how many records existed then.
///
/// Only records appended after the selection can continue it, so rewinding to an
/// earlier record never follows the descendants it abandoned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Selection {
    pub head: RecordId,
    pub seen: u64,
}

impl HistoryRow {
    /// A history first observed at revision 1.
    pub(super) const fn first(source: SourceRef, observation: Option<Observation>) -> Self {
        Self {
            source,
            observation,
            revision: HistoryRevision::new(1),
            last_change: None,
            selected: None,
        }
    }

    /// Moves to the next revision because of `change`.
    pub(super) const fn advance(&mut self, change: HistoryChange) {
        self.revision = HistoryRevision::new(self.revision.get().saturating_add(1));
        self.last_change = Some(change);
    }

    /// Records `content` as the latest observation; whether the transcript changed.
    pub(super) fn observe(&mut self, content: &Content) -> bool {
        let change = self
            .observation
            .as_ref()
            .and_then(|recorded| content.change_since(recorded));
        if let Some(change) = change {
            self.advance(change);
        }
        let first = self.observation.is_none();
        self.observation = Some(content.observation.clone());
        change.is_some() || first
    }

    /// The identity and revision this row describes for `id`.
    pub(super) fn head(&self, id: &HistoryId, change: Option<HistoryChange>) -> HistoryHead {
        HistoryHead::new(id.clone(), self.revision, change)
    }
}

/// A history row, absent before its first binding, and its compare-and-swap version.
#[derive(Debug)]
pub(super) struct StoredHistory {
    pub row: Option<HistoryRow>,
    version: Version,
    key: ResourceKey,
}

impl StoredHistory {
    /// Reads the row of `id` in `runtime`.
    pub(super) fn read(
        snapshot: &Snapshot,
        runtime: RuntimeId,
        id: &HistoryId,
    ) -> Result<Self, BindingError> {
        let key = ResourceKey::new("conversation_history", format!("{runtime}/{}", id.as_str()));
        let (row, version) = match snapshot.resource(&key).map_err(EventError::from)? {
            ResourceLookup::Live { value, version, .. } => {
                (Some(serde_json::from_slice(value.as_bytes())?), version)
            }
            ResourceLookup::Absent => (None, Version::ABSENT),
            _ => return Err(EventError::SnapshotUnavailable.into()),
        };
        Ok(Self { row, version, key })
    }

    /// The compare-and-swap write of `row`.
    pub(super) fn write(&self, row: &HistoryRow) -> Result<CasWrite, BindingError> {
        Ok(CasWrite::new(
            self.key.clone(),
            self.version,
            Some(Payload::from(serde_json::to_vec(row)?)),
        ))
    }
}

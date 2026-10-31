//! `doc.changed` and the event union this lane commits through the coordinator.

use ovis_terminal_protocol::schema::methods::doc::DocEntry;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use serde::{Deserialize, Serialize};

use crate::events::EventData;
use crate::ops::OperationEvent;
use crate::retention::FloorAdvanced;

/// `doc.changed`: a document reached a new version; the value is never carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DocChanged(DocEntry);

impl DocChanged {
    /// The change of `entry.name` to `entry.version`.
    pub const fn new(entry: DocEntry) -> Self {
        Self(entry)
    }

    /// Name and new version.
    pub const fn entry(&self) -> &DocEntry {
        &self.0
    }
}

impl EventData for DocChanged {
    fn event_type(&self) -> &'static str {
        "doc.changed"
    }
}

/// Document changes, retention floors and operation outcomes in one sequence domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum DocumentsRetentionEvent {
    /// A document version changed.
    DocChanged(DocChanged),
    /// A retention floor advanced.
    FloorAdvanced(FloorAdvanced),
    /// The released operation event shape, unwrapped.
    #[serde(untagged)]
    Operation(Box<OperationEvent>),
}

impl From<OperationEvent> for DocumentsRetentionEvent {
    fn from(event: OperationEvent) -> Self {
        Self::Operation(Box::new(event))
    }
}

impl From<DocChanged> for DocumentsRetentionEvent {
    fn from(event: DocChanged) -> Self {
        Self::DocChanged(event)
    }
}

impl From<FloorAdvanced> for DocumentsRetentionEvent {
    fn from(event: FloorAdvanced) -> Self {
        Self::FloorAdvanced(event)
    }
}

impl EventData for DocumentsRetentionEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::DocChanged(event) => event.event_type(),
            Self::FloorAdvanced(event) => event.event_type(),
            Self::Operation(event) => event.event_type(),
        }
    }

    fn at_sequence(&mut self, seq: EventSeq) {
        match self {
            Self::Operation(event) => event.at_sequence(seq),
            Self::FloorAdvanced(event) => event.at_sequence(seq),
            Self::DocChanged(_) => {}
        }
    }
}

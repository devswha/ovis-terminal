//! Change results, launch outcomes and the revisioned layout events.

use ovis_terminal_protocol::errors::ErrorData;
use ovis_terminal_protocol::schema::ids::ClientId;
use ovis_terminal_protocol::schema::scalar::{EventSeq, Revision};
use serde::{Deserialize, Serialize};

use super::NoOpReason;
use super::geometry::TabGeometry;
use crate::events::{EventData, ResourceId};
use crate::ops::OperationEvent;
use crate::registry::{PaneId, TabId, TerminalId, WorkspaceId};
use crate::terminals::LaunchState;

/// Launch result, reported separately from the resources a creation committed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[non_exhaustive]
pub enum LaunchOutcome {
    /// The edit did not ask for a program.
    NotRequested,
    /// A terminal was admitted and bound to the created pane.
    Launched {
        /// Terminal bound to the pane.
        terminal: TerminalId,
        /// Spawn progress when the launch call returned.
        spawn: LaunchState,
    },
    /// No terminal was admitted; the created resources remain.
    Rejected {
        /// Typed launch rejection.
        error: ErrorData,
    },
}

/// Result of every layout edit or selection change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Change {
    /// Layout revision after the request.
    pub revision: Revision,
    /// Why nothing changed, when nothing did.
    pub no_op: Option<NoOpReason>,
    /// Resources this request created.
    pub created: Vec<ResourceId>,
    /// Resources this request removed.
    pub removed: Vec<ResourceId>,
    /// Surviving resources whose placement or shape changed.
    pub changed: Vec<ResourceId>,
    /// Terminals no longer shown by any pane; they keep running.
    pub released_terminals: Vec<TerminalId>,
    /// Clients whose selection named a removed resource.
    pub cleared_selections: Vec<ClientId>,
    /// Resulting geometry of every surviving affected tab.
    pub geometry: Vec<TabGeometry>,
    /// Launch outcome, separate from creation.
    pub launch: LaunchOutcome,
}

/// Why a client's selection changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SelectionCause {
    /// The client asked for this selection.
    Requested,
    /// A selected resource was removed; the reference was cleared, not retargeted.
    TargetRemoved,
}

/// One client's resulting selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SelectionChanged {
    /// Client whose selection changed.
    pub client: ClientId,
    /// Selected workspace.
    pub workspace: Option<WorkspaceId>,
    /// Selected tab.
    pub tab: Option<TabId>,
    /// Selected pane.
    pub pane: Option<PaneId>,
    /// Why it changed.
    pub cause: SelectionCause,
}

/// A committed structural change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LayoutChanged {
    /// Layout revision, stamped at commit.
    pub revision: Revision,
    /// Created resources.
    pub created: Vec<ResourceId>,
    /// Removed resources.
    pub removed: Vec<ResourceId>,
    /// Changed resources.
    pub changed: Vec<ResourceId>,
}

/// A terminal was bound to a pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PaneBound {
    /// Layout revision, stamped at commit.
    pub revision: Revision,
    /// Pane.
    pub pane: PaneId,
    /// Terminal now shown by the pane.
    pub terminal: TerminalId,
}

/// Layout events and operation outcomes share the coordinator sequence domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum LayoutEvent {
    /// Structure, placement or geometry changed.
    Changed(LayoutChanged),
    /// One client's selection changed.
    Selection(SelectionChanged),
    /// A launched terminal was bound to its pane.
    Bound(PaneBound),
    /// The released operation event's unwrapped representation.
    #[serde(untagged)]
    Operation(Box<OperationEvent>),
}

impl From<OperationEvent> for LayoutEvent {
    fn from(event: OperationEvent) -> Self {
        Self::Operation(Box::new(event))
    }
}

impl EventData for LayoutEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Changed(_) => "layout.changed",
            Self::Selection(_) => "selection.changed",
            Self::Bound(_) => "layout.pane_bound",
            Self::Operation(event) => event.event_type(),
        }
    }

    fn at_sequence(&mut self, seq: EventSeq) {
        match self {
            Self::Changed(LayoutChanged { revision, .. })
            | Self::Bound(PaneBound { revision, .. }) => *revision = Revision::new(seq.get()),
            Self::Operation(event) => event.at_sequence(seq),
            Self::Selection(_) => {}
        }
    }
}

impl SelectionChanged {
    pub(super) fn affected(&self) -> Vec<ResourceId> {
        let mut affected: Vec<ResourceId> = Vec::new();
        affected.extend(self.workspace.map(ResourceId::Workspace));
        affected.extend(self.tab.map(ResourceId::Tab));
        affected.extend(self.pane.map(ResourceId::Pane));
        affected
    }
}

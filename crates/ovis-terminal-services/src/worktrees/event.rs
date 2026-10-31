//! The committed change and its `worktree.changed` event.

use ovis_terminal_core::events::{Commit, EventData, ResourceId};
use ovis_terminal_core::registry::{MachineId, WorkspaceId};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids;
use ovis_terminal_protocol::schema::methods::worktree::{WorktreeChange, WorktreeRecord};
use ovis_terminal_protocol::schema::scalar::EventSeq;
use serde::{Deserialize, Serialize};

/// Event data of the worktree service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WorktreeEvent {
    /// A checkout was created, opened in a workspace or removed.
    Changed(WorktreeRecord),
}

impl EventData for WorktreeEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Changed(_) => "worktree.changed",
        }
    }
}

/// An applied change before its event commits.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Change {
    /// The checkout after the change; a removed checkout keeps its last record.
    pub record: WorktreeRecord,
    /// Workspaces this change opened or closed.
    pub workspaces: Vec<WorkspaceId>,
}

impl Change {
    /// The `worktree.changed` commit of operation `op`, naming the machine and every
    /// touched workspace.
    ///
    /// # Errors
    ///
    /// Returns the encoding failure of the operation identity.
    pub fn commit<D: From<WorktreeEvent>>(
        &self,
        op: &OpId,
        machine: MachineId,
    ) -> Result<Commit<D>, serde_json::Error> {
        let op: String = serde_json::from_value(serde_json::to_value(op)?)?;
        let mut resources = vec![ResourceId::Machine(machine)];
        resources.extend(self.workspaces.iter().copied().map(ResourceId::Workspace));
        let event = WorktreeEvent::Changed(self.record.clone());
        Ok(Commit::new(
            ovis_terminal_store::OpId::new(op),
            D::from(event),
            resources,
        ))
    }

    /// The wire result once the event committed at `seq`.
    #[must_use]
    pub fn into_wire(self, seq: EventSeq) -> WorktreeChange {
        let workspaces = self.workspaces.iter();
        let affected = workspaces.map(|id| ids::WorkspaceId::new(id.to_string()));
        WorktreeChange::new(self.record, affected.collect(), seq)
    }
}

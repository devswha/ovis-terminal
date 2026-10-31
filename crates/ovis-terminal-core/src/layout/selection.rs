//! Per-client selections: set only by their own client, cleared only on removal.

use ovis_terminal_protocol::schema::ids::ClientId;
use ovis_terminal_protocol::schema::scalar::Revision;
use serde::{Deserialize, Serialize};

use super::LayoutError;
use super::model::{Layout, MAX_CLIENTS, Selection, label};
use super::records::{SelectionCause, SelectionChanged};
use crate::events::ResourceId;
use crate::registry::{PaneId, TabId, WorkspaceId};

/// Parameters of `selection.set`; only the named client's selection changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SelectRequest {
    /// Client whose selection changes.
    pub client: ClientId,
    /// Workspace to select.
    pub workspace: Option<WorkspaceId>,
    /// Tab to select; must belong to the workspace when both are named.
    pub tab: Option<TabId>,
    /// Pane to select; must belong to the tab when both are named.
    pub pane: Option<PaneId>,
    /// Layout revision the request was computed against.
    pub expect_revision: Option<Revision>,
}

impl SelectRequest {
    /// Selects explicit identities for one client.
    pub const fn new(
        client: ClientId,
        (workspace, tab, pane): (Option<WorkspaceId>, Option<TabId>, Option<PaneId>),
    ) -> Self {
        Self {
            client,
            workspace,
            tab,
            pane,
            expect_revision: None,
        }
    }
}

fn changed(selection: &Selection, cause: SelectionCause) -> SelectionChanged {
    SelectionChanged {
        client: selection.client.clone(),
        workspace: selection.workspace,
        tab: selection.tab,
        pane: selection.pane,
        cause,
    }
}

fn references(selection: &Selection) -> Vec<ResourceId> {
    let mut ids = Vec::new();
    ids.extend(selection.workspace.map(ResourceId::Workspace));
    ids.extend(selection.tab.map(ResourceId::Tab));
    ids.extend(selection.pane.map(ResourceId::Pane));
    ids
}

/// Clears references to removed resources; nothing is retargeted to a neighbor.
pub(super) fn clear_removed(
    layout: &mut Layout,
    removed: &[ResourceId],
) -> Vec<(SelectionChanged, Vec<ResourceId>)> {
    let gone = |id: ResourceId| removed.contains(&id);
    let mut events = Vec::new();
    for selection in &mut layout.selections {
        let before = references(selection);
        if !before.iter().any(|id| gone(id.clone())) {
            continue;
        }
        selection.workspace = selection
            .workspace
            .filter(|id| !gone(ResourceId::Workspace(*id)));
        selection.tab = selection.tab.filter(|id| !gone(ResourceId::Tab(*id)));
        selection.pane = selection.pane.filter(|id| !gone(ResourceId::Pane(*id)));
        events.push((changed(selection, SelectionCause::TargetRemoved), before));
    }
    events
}

/// Replaces one client's selection after validating every named identity.
pub(super) fn select(
    layout: &mut Layout,
    request: &SelectRequest,
) -> Result<Option<SelectionChanged>, LayoutError> {
    label(request.client.as_str(), "client")?;
    if let Some(workspace) = request.workspace {
        layout.workspace_at(workspace)?;
    }
    if let Some(tab) = request.tab {
        let (index, _) = layout.tab_at(tab)?;
        let owner = layout.workspaces.get(index).map(|workspace| workspace.id);
        if request.workspace.is_some() && owner != request.workspace {
            return Err(LayoutError::InvalidParams { field: "tab" });
        }
    }
    if let Some(pane) = request.pane {
        let at = layout.pane_at(pane)?;
        if request
            .tab
            .is_some_and(|tab| layout.tab(at).is_ok_and(|t| t.id != tab))
        {
            return Err(LayoutError::InvalidParams { field: "pane" });
        }
    }
    let next = Selection {
        client: request.client.clone(),
        workspace: request.workspace,
        tab: request.tab,
        pane: request.pane,
    };
    let existing = layout
        .selections
        .iter()
        .position(|selection| selection.client == request.client);
    let count = layout.selections.len();
    match existing.and_then(|index| layout.selections.get_mut(index)) {
        Some(current) if *current == next => return Ok(None),
        Some(current) => *current = next.clone(),
        None if count >= MAX_CLIENTS => return Err(LayoutError::LimitExceeded),
        None => layout.selections.push(next.clone()),
    }
    Ok(Some(changed(&next, SelectionCause::Requested)))
}

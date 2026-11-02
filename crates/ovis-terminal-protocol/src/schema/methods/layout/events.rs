//! Data of `layout.changed` and `selection.changed`: the record a subscriber receives,
//! naming the operation that caused it.
//!
//! Both extend the released data shapes (`LayoutTree` and `Selection`) with fields, so a
//! subscriber written against the released document still decodes them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{LayoutResource, Selection, WorkspaceRecord};
use crate::errors::OpId;
use crate::schema::ids::{ClientId, PaneId, TabId, TerminalId, WorkspaceId};
use crate::schema::scalar::{OpenEnum, Revision};

/// Data of `layout.changed`: the layout after one operation and what that operation changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct LayoutChangedData {
    /// Layout revision the published layout is at.
    pub revision: Revision,
    /// Workspaces in order, after the change.
    pub workspaces: Vec<WorkspaceRecord>,
    /// Per-client selections, after the change.
    pub selections: Vec<Selection>,
    /// Operation that committed the change.
    pub op_id: OpId,
    /// Workspaces, tabs, panes and nodes the operation created.
    pub created: Vec<LayoutResource>,
    /// Resources the operation removed.
    pub removed: Vec<LayoutResource>,
    /// Surviving resources whose placement or shape changed.
    pub changed: Vec<LayoutResource>,
}

/// Why a client's selection changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SelectionChangeCause {
    /// The client asked for this selection.
    Requested,
    /// A selected resource was removed; the reference was cleared, not retargeted.
    TargetRemoved,
}

/// Data of `selection.changed`: one client's new selection and the operation behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct SelectionChangedData {
    /// Client the selection belongs to.
    pub client_id: ClientId,
    /// Selected workspace.
    pub workspace_id: Option<WorkspaceId>,
    /// Selected tab.
    pub tab_id: Option<TabId>,
    /// Selected pane.
    pub pane_id: Option<PaneId>,
    /// Terminal the selected pane shows, if any.
    pub terminal_id: Option<TerminalId>,
    /// Operation that changed the selection.
    pub op_id: OpId,
    /// Layout revision after the change.
    pub revision: Revision,
    /// Why it changed.
    pub cause: OpenEnum<SelectionChangeCause>,
}

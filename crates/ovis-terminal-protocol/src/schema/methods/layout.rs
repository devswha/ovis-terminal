//! Thin layout (`layout.*`) and per-client selection (`selection.*`) by explicit ID.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::OpId;
use crate::schema::ids::{ClientId, PaneId, SplitNodeId, TabId, TerminalId, WorkspaceId};
use crate::schema::method::{MethodSpec, method};
use crate::schema::methods::terminal::{LaunchProgram, SpawnState};
use crate::schema::scalar::{EventSeq, OpenEnum, Revision};

/// Direction of a split.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SplitAxis {
    /// Children side by side.
    Horizontal,
    /// Children stacked.
    Vertical,
}

/// One node of a tab's split tree; node IDs never derive from tree position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct SplitNode {
    /// Stable node identity.
    pub node_id: SplitNodeId,
    /// Split direction, for interior nodes.
    pub axis: Option<OpenEnum<SplitAxis>>,
    /// Ratio of the first child in thousandths, for interior nodes.
    pub ratio_permille: Option<u16>,
    /// Child nodes, for interior nodes.
    pub children: Vec<SplitNodeId>,
    /// Pane, for leaf nodes.
    pub pane_id: Option<PaneId>,
}

/// One pane bound to a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct PaneRecord {
    /// Stable pane identity.
    pub pane_id: PaneId,
    /// Terminal shown in the pane.
    pub terminal_id: Option<TerminalId>,
    /// Whether the pane is zoomed.
    pub zoomed: bool,
}

/// One tab and its split tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TabRecord {
    /// Stable tab identity.
    pub tab_id: TabId,
    /// Display name.
    pub name: Option<String>,
    /// Root of the split tree.
    pub root: SplitNodeId,
    /// Split nodes.
    pub nodes: Vec<SplitNode>,
    /// Panes.
    pub panes: Vec<PaneRecord>,
}

/// One workspace and its tabs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct WorkspaceRecord {
    /// Stable workspace identity.
    pub workspace_id: WorkspaceId,
    /// Display name.
    pub name: String,
    /// Tabs in order.
    pub tabs: Vec<TabRecord>,
}

/// One client's selection; each client keeps its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Selection {
    /// Client the selection belongs to.
    pub client_id: ClientId,
    /// Selected workspace.
    pub workspace_id: Option<WorkspaceId>,
    /// Selected tab.
    pub tab_id: Option<TabId>,
    /// Selected pane.
    pub pane_id: Option<PaneId>,
}

/// Full layout at one revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct LayoutTree {
    /// Layout revision.
    pub revision: Revision,
    /// Workspaces in order.
    pub workspaces: Vec<WorkspaceRecord>,
    /// Per-client selections.
    pub selections: Vec<Selection>,
}

/// A workspace, tab, pane or split node named by its stable identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum LayoutResource {
    /// A workspace.
    WorkspaceId(WorkspaceId),
    /// A tab.
    TabId(TabId),
    /// A pane.
    PaneId(PaneId),
    /// A split node.
    NodeId(SplitNodeId),
}

/// Why a layout mutation changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NoOpReason {
    /// The layout already had the requested shape.
    AlreadyApplied,
    /// The request would need clamping, which is never silent.
    WouldClamp,
}

/// Result of every layout mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct LayoutChange {
    /// Layout after the mutation.
    pub layout: LayoutTree,
    /// Why nothing changed, when nothing did.
    pub no_op: Option<OpenEnum<NoOpReason>>,
    /// Workspaces, tabs, panes and nodes this mutation created.
    pub created: Vec<LayoutResource>,
    /// Resources this mutation removed.
    pub removed: Vec<LayoutResource>,
    /// Launch outcome, reported separately from creation.
    pub spawn: Option<OpenEnum<SpawnState>>,
    /// Sequence of the commit event.
    pub seq: EventSeq,
}

/// Destructive scope a close request explicitly accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CloseScope {
    /// Only the named resource; refuse if others would be affected.
    Target,
    /// The named resource and its linked group.
    Group,
}

/// A layout mutation; every variant names its resources by stable ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "op", deny_unknown_fields)]
#[non_exhaustive]
pub enum LayoutEdit {
    /// Create a workspace with one tab, optionally launching a program.
    CreateWorkspace {
        /// Name.
        name: String,
        /// Program for the first pane.
        program: Option<LaunchProgram>,
    },
    /// Create a tab in a workspace.
    CreateTab {
        /// Workspace.
        workspace_id: WorkspaceId,
        /// Program for the first pane.
        program: Option<LaunchProgram>,
    },
    /// Split a pane.
    SplitPane {
        /// Pane to split.
        pane_id: PaneId,
        /// Direction.
        axis: SplitAxis,
        /// Program for the new pane.
        program: Option<LaunchProgram>,
    },
    /// Close a workspace, tab or pane with explicit scope.
    Close {
        /// Workspace, tab or pane.
        resource: LayoutResource,
        /// Accepted destructive scope.
        scope: CloseScope,
    },
    /// Set a split node's ratio.
    SetRatio {
        /// Interior node.
        node_id: SplitNodeId,
        /// Ratio of the first child in thousandths.
        ratio_permille: u16,
    },
    /// Swap two panes.
    SwapPanes {
        /// First pane.
        first: PaneId,
        /// Second pane.
        second: PaneId,
    },
    /// Zoom or unzoom a pane.
    ZoomPane {
        /// Pane.
        pane_id: PaneId,
        /// Zoomed after the edit.
        zoomed: bool,
    },
    /// Move a tab to a workspace position.
    MoveTab {
        /// Tab.
        tab_id: TabId,
        /// Destination workspace.
        workspace_id: WorkspaceId,
        /// Destination index.
        index: u32,
    },
    /// Rename a workspace or tab.
    Rename {
        /// Workspace or tab.
        resource: LayoutResource,
        /// New name.
        name: String,
    },
}

/// Parameters of `layout.edit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LayoutEditParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Layout revision the edit was computed against.
    pub expect_revision: Option<Revision>,
    /// Edit.
    pub edit: LayoutEdit,
}

/// Parameters of `selection.set`; only the named client's selection changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SelectionSetParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Client whose selection changes.
    pub client_id: ClientId,
    /// Workspace to select.
    pub workspace_id: Option<WorkspaceId>,
    /// Tab to select.
    pub tab_id: Option<TabId>,
    /// Pane to select.
    pub pane_id: Option<PaneId>,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"layout.get", Query, Untargeted, crate::schema::common::NoParams => LayoutTree,
    "Read the layout and every client's selection at one revision.", []},
    method! {"layout.edit", Mutation, Lookup, LayoutEditParams => LayoutChange,
    "Apply one explicit-ID layout edit with created IDs and launch outcome.", [RevisionMismatch, InvalidGeometry, ScopeRequired, SpawnFailed]},
    method! {"selection.set", Mutation, Lookup, SelectionSetParams => LayoutChange,
    "Change one client's selection; other clients keep theirs.", [RevisionMismatch]},
];

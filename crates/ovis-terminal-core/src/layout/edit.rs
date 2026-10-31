//! Explicit-ID layout edits and their pure effect on a layout value.

use serde::{Deserialize, Serialize};

use super::model::{Axis, Layout};
use super::{LayoutError, arrange, close, structure};
use crate::events::ResourceId;
use crate::registry::{PaneId, SplitId, TabId, TerminalId, WorkspaceId};

/// A workspace, tab or pane named for a close.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Target {
    /// A workspace.
    Workspace(WorkspaceId),
    /// A tab.
    Tab(TabId),
    /// A pane.
    Pane(PaneId),
}

/// Destructive scope a close explicitly accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CloseScope {
    /// Only the named resource and what it contains.
    Target,
    /// The named workspace and every workspace linked to it.
    Group,
}

/// Why an accepted edit changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NoOpReason {
    /// The layout already had the requested shape.
    AlreadyApplied,
}

/// One layout mutation; every variant names its resources by stable ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Edit {
    /// Create a workspace holding one tab with one pane.
    CreateWorkspace {
        /// Display name.
        name: String,
        /// Existing workspace whose group the new workspace joins.
        group_with: Option<WorkspaceId>,
    },
    /// Create a tab with one pane at the end of a workspace.
    CreateTab {
        /// Workspace.
        workspace: WorkspaceId,
    },
    /// Split a pane; the new pane becomes the second child.
    Split {
        /// Pane to split.
        pane: PaneId,
        /// Direction.
        axis: Axis,
        /// First-child share in thousandths.
        ratio_permille: u16,
    },
    /// Close a workspace, tab or pane with explicit scope.
    Close {
        /// Resource to close.
        target: Target,
        /// Accepted destructive scope.
        scope: CloseScope,
    },
    /// Move a tab, with its whole split tree, to a workspace position.
    MoveTab {
        /// Tab.
        tab: TabId,
        /// Destination workspace.
        workspace: WorkspaceId,
        /// Destination index after the tab leaves its source.
        index: u32,
    },
    /// Move a pane into a new tab at the end of a workspace.
    MovePane {
        /// Pane.
        pane: PaneId,
        /// Destination workspace.
        workspace: WorkspaceId,
    },
    /// Exchange the positions of two panes.
    Swap {
        /// First pane.
        first: PaneId,
        /// Second pane.
        second: PaneId,
    },
    /// Zoom or unzoom a pane without changing any selection.
    Zoom {
        /// Pane.
        pane: PaneId,
        /// Zoomed after the edit.
        zoomed: bool,
    },
    /// Set a split's first-child share.
    SetRatio {
        /// Split node.
        split: SplitId,
        /// First-child share in thousandths.
        ratio_permille: u16,
    },
    /// Move a split's divider by an amount of the whole tab extent.
    Resize {
        /// Split node.
        split: SplitId,
        /// Signed divider movement in tab parts per million.
        delta_ppm: i32,
    },
}

impl Edit {
    /// Whether this edit creates a pane a program may be launched into.
    pub const fn creates_pane(&self) -> bool {
        matches!(
            self,
            Self::CreateWorkspace { .. } | Self::CreateTab { .. } | Self::Split { .. }
        )
    }

    pub(super) const fn needs(&self) -> &'static [Family] {
        match self {
            Self::CreateWorkspace { .. } => &[Family::Workspace, Family::Tab, Family::Pane],
            Self::CreateTab { .. } => &[Family::Tab, Family::Pane],
            Self::Split { .. } => &[Family::Pane, Family::Split],
            Self::MovePane { .. } => &[Family::Tab],
            Self::Close { .. }
            | Self::MoveTab { .. }
            | Self::Swap { .. }
            | Self::Zoom { .. }
            | Self::SetRatio { .. }
            | Self::Resize { .. } => &[],
        }
    }
}

/// Identity family an edit allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    Workspace,
    Tab,
    Pane,
    Split,
}

/// Identities reserved for this edit inside its commit unit.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Fresh {
    pub workspace: Option<WorkspaceId>,
    pub tab: Option<TabId>,
    pub pane: Option<PaneId>,
    pub split: Option<SplitId>,
}

/// Every resource an edit created, removed or changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Effect {
    pub created: Vec<ResourceId>,
    pub removed: Vec<ResourceId>,
    pub changed: Vec<ResourceId>,
    pub released: Vec<TerminalId>,
    pub scopes: Vec<WorkspaceId>,
    pub tabs: Vec<TabId>,
    pub no_op: Option<NoOpReason>,
}

impl Effect {
    pub(super) fn unchanged() -> Self {
        Self {
            no_op: Some(NoOpReason::AlreadyApplied),
            ..Self::default()
        }
    }

    pub(super) fn affected(&self) -> Vec<ResourceId> {
        let mut affected = Vec::new();
        for id in self
            .created
            .iter()
            .chain(&self.removed)
            .chain(&self.changed)
            .cloned()
            .chain(self.released.iter().copied().map(ResourceId::Terminal))
        {
            if !affected.contains(&id) {
                affected.push(id);
            }
        }
        affected
    }
}

pub(super) const fn missing() -> LayoutError {
    LayoutError::Event(crate::events::EventError::InvalidRecord)
}

/// Applies one edit to `layout`, never touching selections or focus.
pub(super) fn apply(layout: &mut Layout, edit: &Edit, fresh: Fresh) -> Result<Effect, LayoutError> {
    match edit {
        Edit::CreateWorkspace { name, group_with } => {
            structure::create_workspace(layout, (name, *group_with), fresh)
        }
        Edit::CreateTab { workspace } => structure::create_tab(layout, *workspace, fresh),
        Edit::Split {
            pane,
            axis,
            ratio_permille,
        } => structure::split(layout, (*pane, *axis, *ratio_permille), fresh),
        Edit::Close { target, scope } => close::close(layout, *target, *scope),
        Edit::MoveTab {
            tab,
            workspace,
            index,
        } => structure::move_tab(layout, *tab, (*workspace, *index)),
        Edit::MovePane { pane, workspace } => {
            structure::move_pane(layout, (*pane, *workspace), fresh)
        }
        Edit::Swap { first, second } => arrange::swap(layout, *first, *second),
        Edit::Zoom { pane, zoomed } => arrange::zoom(layout, *pane, *zoomed),
        Edit::SetRatio {
            split,
            ratio_permille,
        } => arrange::set_ratio(layout, *split, *ratio_permille),
        Edit::Resize { split, delta_ppm } => arrange::resize(layout, *split, *delta_ppm),
    }
}

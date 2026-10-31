//! Layout records: workspaces, tabs, split nodes, panes and per-client selections.

use ovis_terminal_protocol::schema::ids::ClientId;
use ovis_terminal_protocol::schema::scalar::Revision;
use serde::{Deserialize, Serialize};

use super::{GeometryReason, GeometryRejection, LayoutError};
use crate::events::{EventError, ResourceId};
use crate::registry::{PaneId, SplitId, TabId, TerminalId, WorkspaceId};

pub(super) const MAX_WORKSPACES: usize = 128;
pub(super) const MAX_TABS: usize = 32;
pub(super) const MAX_PANES: usize = 32;
pub(super) const MAX_CLIENTS: usize = 64;
const MAX_LABEL_BYTES: usize = 256;

/// Direction in which a split places its two children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Axis {
    /// Children side by side; the split divides the width.
    Horizontal,
    /// Children stacked; the split divides the height.
    Vertical,
}

/// Share of a split given to its first child, in thousandths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct Ratio(u16);

impl Ratio {
    /// Smallest supported first-child share.
    pub const MIN: u16 = 100;
    /// Largest supported first-child share.
    pub const MAX: u16 = 900;

    /// Accepts a share inside the supported range.
    ///
    /// # Errors
    ///
    /// Returns `invalid_geometry` with the exact range; the value is never clamped.
    pub fn new(permille: u16) -> Result<Self, LayoutError> {
        if (Self::MIN..=Self::MAX).contains(&permille) {
            return Ok(Self(permille));
        }
        Err(LayoutError::InvalidGeometry(GeometryRejection {
            reason: GeometryReason::OutOfRange,
            requested: i64::from(permille),
            min: i64::from(Self::MIN),
            max: i64::from(Self::MAX),
            step: 1,
        }))
    }

    /// Share in thousandths.
    pub const fn permille(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for Ratio {
    type Error = LayoutError;

    fn try_from(permille: u16) -> Result<Self, Self::Error> {
        Self::new(permille)
    }
}

impl From<Ratio> for u16 {
    fn from(ratio: Ratio) -> Self {
        ratio.0
    }
}

/// A slot of the split tree: a pane leaf or an interior split node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Child {
    /// A pane leaf.
    Pane(PaneId),
    /// An interior split node.
    Split(SplitId),
}

/// An interior node; its identity never derives from its tree position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Split {
    /// Stable split identity.
    pub id: SplitId,
    /// Split direction.
    pub axis: Axis,
    /// Share of the first child.
    pub ratio: Ratio,
    /// First (left or top) child.
    pub first: Child,
    /// Second (right or bottom) child.
    pub second: Child,
}

/// A pane and the terminal bound to it, if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Pane {
    /// Stable pane identity.
    pub id: PaneId,
    /// Terminal shown in the pane.
    pub terminal: Option<TerminalId>,
}

/// One tab: a split tree that may be empty, never closed implicitly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Tab {
    /// Stable tab identity.
    pub id: TabId,
    /// Root slot; absent once the last pane left.
    pub root: Option<Child>,
    /// Interior split nodes.
    pub splits: Vec<Split>,
    /// Panes in creation order.
    pub panes: Vec<Pane>,
    /// Pane shown alone, when zoomed.
    pub zoomed: Option<PaneId>,
}

/// One workspace; linked workspaces share a group anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Workspace {
    /// Stable workspace identity.
    pub id: WorkspaceId,
    /// Display name.
    pub name: String,
    /// Group anchor shared by linked workspaces.
    pub group: Option<WorkspaceId>,
    /// Tabs in order.
    pub tabs: Vec<Tab>,
}

/// One client's selection; changed only by that client's explicit request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Selection {
    /// Client the selection belongs to.
    pub client: ClientId,
    /// Selected workspace.
    pub workspace: Option<WorkspaceId>,
    /// Selected tab.
    pub tab: Option<TabId>,
    /// Selected pane.
    pub pane: Option<PaneId>,
}

/// Complete layout and every client's selection at one revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Layout {
    /// Revision of the last committed layout change.
    pub revision: Revision,
    /// Workspaces in order.
    pub workspaces: Vec<Workspace>,
    /// Per-client selections.
    pub selections: Vec<Selection>,
}

/// Position of a tab: workspace index and tab index.
pub(super) type TabAt = (usize, usize);

impl Layout {
    pub(super) const fn empty() -> Self {
        Self {
            revision: Revision::new(0),
            workspaces: Vec::new(),
            selections: Vec::new(),
        }
    }

    pub(super) fn workspace_at(&self, id: WorkspaceId) -> Result<usize, LayoutError> {
        self.workspaces
            .iter()
            .position(|workspace| workspace.id == id)
            .ok_or(LayoutError::NotFound {
                resource: ResourceId::Workspace(id),
            })
    }

    pub(super) fn tab_at(&self, id: TabId) -> Result<TabAt, LayoutError> {
        self.find(|tab| tab.id == id).ok_or(LayoutError::NotFound {
            resource: ResourceId::Tab(id),
        })
    }

    pub(super) fn pane_at(&self, id: PaneId) -> Result<TabAt, LayoutError> {
        self.find(|tab| tab.panes.iter().any(|pane| pane.id == id))
            .ok_or(LayoutError::NotFound {
                resource: ResourceId::Pane(id),
            })
    }

    pub(super) fn split_at(&self, id: SplitId) -> Result<TabAt, LayoutError> {
        self.find(|tab| tab.splits.iter().any(|split| split.id == id))
            .ok_or(LayoutError::NotFound {
                resource: ResourceId::Split(id),
            })
    }

    pub(super) fn tab_mut(&mut self, (workspace, tab): TabAt) -> Result<&mut Tab, LayoutError> {
        self.workspaces
            .get_mut(workspace)
            .and_then(|workspace| workspace.tabs.get_mut(tab))
            .ok_or(LayoutError::Event(EventError::InvalidRecord))
    }

    pub(super) fn tab(&self, (workspace, tab): TabAt) -> Result<&Tab, LayoutError> {
        self.workspaces
            .get(workspace)
            .and_then(|workspace| workspace.tabs.get(tab))
            .ok_or(LayoutError::Event(EventError::InvalidRecord))
    }

    fn find(&self, matches: impl Fn(&Tab) -> bool) -> Option<TabAt> {
        self.workspaces
            .iter()
            .enumerate()
            .find_map(|(index, workspace)| {
                let tab = workspace.tabs.iter().position(&matches)?;
                Some((index, tab))
            })
    }
}

pub(super) fn label(name: &str, field: &'static str) -> Result<String, LayoutError> {
    if name.is_empty() || name.len() > MAX_LABEL_BYTES || name.chars().any(char::is_control) {
        return Err(LayoutError::InvalidParams { field });
    }
    Ok(name.to_owned())
}

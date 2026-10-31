//! Resource scopes of grants and the resolved targets they are checked against.

use ovis_terminal_protocol::schema::ids::{
    DocName, MachineId, PaneId, PluginId, TabId, TerminalId, WorkspaceId,
};
use ovis_terminal_protocol::schema::scalar::Incarnation;
use serde::{Deserialize, Serialize};

use crate::permission::Permission;

/// The resource a grant reaches on its machine.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum ScopeResource {
    /// Every resource of the machine.
    Machine,
    /// One workspace and everything placed in it.
    Workspace {
        /// The workspace.
        id: WorkspaceId,
    },
    /// One tab and everything placed in it.
    Tab {
        /// The tab.
        id: TabId,
    },
    /// One pane and the terminal placed in it.
    Pane {
        /// The pane.
        id: PaneId,
    },
    /// One terminal, across its incarnations.
    Terminal {
        /// The terminal.
        id: TerminalId,
    },
    /// One shared document.
    Document {
        /// The document.
        name: DocName,
    },
    /// One plugin.
    Plugin {
        /// The plugin.
        id: PluginId,
    },
}

/// A resource on one machine.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceScope {
    machine: MachineId,
    resource: ScopeResource,
}

impl ResourceScope {
    /// Scopes `resource` on `machine`.
    #[must_use]
    pub const fn new(machine: MachineId, resource: ScopeResource) -> Self {
        Self { machine, resource }
    }

    fn covers(&self, target: &ResolvedTarget) -> bool {
        self.machine == target.machine && self.resource_covers(&target.resource)
    }

    fn contains(&self, inner: &Self) -> bool {
        self.machine == inner.machine
            && (self.resource == ScopeResource::Machine || self.resource == inner.resource)
    }

    fn resource_covers(&self, target: &TargetResource) -> bool {
        match &self.resource {
            ScopeResource::Machine => true,
            ScopeResource::Workspace { id } => {
                target.placement().is_some_and(|at| at.workspace == *id)
                    || matches!(target, TargetResource::Workspace { workspace } if workspace == id)
            }
            ScopeResource::Tab { id } => target.placement().is_some_and(|at| at.tab == *id),
            ScopeResource::Pane { id } => target.placement().is_some_and(|at| at.pane == *id),
            ScopeResource::Terminal { id } => {
                matches!(target, TargetResource::Terminal { terminal, .. } if terminal == id)
            }
            ScopeResource::Document { name } => {
                matches!(target, TargetResource::Document { name: target_name } if target_name == name)
            }
            ScopeResource::Plugin { id } => {
                matches!(target, TargetResource::Plugin { plugin } if plugin == id)
            }
        }
    }
}

/// One permission over one resource scope.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    permission: Permission,
    scope: ResourceScope,
}

impl Grant {
    /// Grants `permission` over `scope`.
    #[must_use]
    pub const fn new(permission: Permission, scope: ResourceScope) -> Self {
        Self { permission, scope }
    }

    pub(super) fn allows(&self, permission: Permission, target: &ResolvedTarget) -> bool {
        self.permission == permission && self.scope.covers(target)
    }

    pub(super) fn includes(&self, inner: &Self) -> bool {
        self.permission == inner.permission && self.scope.contains(&inner.scope)
    }

    pub(super) const fn machine(&self) -> &MachineId {
        &self.scope.machine
    }

    pub(super) const fn permission(&self) -> Permission {
        self.permission
    }
}

/// Where a terminal sits in the layout.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Placement {
    workspace: WorkspaceId,
    tab: TabId,
    pane: PaneId,
}

impl Placement {
    /// A terminal placed in `pane` of `tab` in `workspace`.
    #[must_use]
    pub const fn new(workspace: WorkspaceId, tab: TabId, pane: PaneId) -> Self {
        Self {
            workspace,
            tab,
            pane,
        }
    }
}

/// The resource a request resolved to, with the ancestry scopes are checked against.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TargetResource {
    /// The machine itself.
    Machine,
    /// A workspace.
    Workspace {
        /// The workspace.
        workspace: WorkspaceId,
    },
    /// One incarnation of a terminal.
    Terminal {
        /// Where the terminal sits; `None` for a terminal outside the layout.
        placement: Option<Placement>,
        /// The terminal.
        terminal: TerminalId,
        /// The incarnation the request resolved to.
        incarnation: Incarnation,
    },
    /// A shared document.
    Document {
        /// The document.
        name: DocName,
    },
    /// A plugin.
    Plugin {
        /// The plugin.
        plugin: PluginId,
    },
}

impl TargetResource {
    const fn placement(&self) -> Option<&Placement> {
        match self {
            Self::Terminal {
                placement: Some(placement),
                ..
            } => Some(placement),
            Self::Terminal {
                placement: None, ..
            }
            | Self::Machine
            | Self::Workspace { .. }
            | Self::Document { .. }
            | Self::Plugin { .. } => None,
        }
    }
}

/// A fully resolved target: the resource and the machine that owns it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedTarget {
    machine: MachineId,
    resource: TargetResource,
}

impl ResolvedTarget {
    /// The resolved `resource` on `machine`.
    #[must_use]
    pub const fn new(machine: MachineId, resource: TargetResource) -> Self {
        Self { machine, resource }
    }
}

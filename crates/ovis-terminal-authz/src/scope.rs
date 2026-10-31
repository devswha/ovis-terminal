//! Resource scopes of grants and the resolved targets they are checked against.

use serde::{Deserialize, Serialize};

use crate::family::IdentityFamily;
use crate::permission::Permission;

/// The resource a grant reaches on its machine.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    deny_unknown_fields,
    bound = ""
)]
#[non_exhaustive]
pub enum ScopeResource<F: IdentityFamily> {
    /// Every resource of the machine.
    Machine,
    /// One workspace and everything placed in it.
    Workspace {
        /// The workspace.
        id: F::Workspace,
    },
    /// One tab and everything placed in it.
    Tab {
        /// The tab.
        id: F::Tab,
    },
    /// One pane and the terminal placed in it.
    Pane {
        /// The pane.
        id: F::Pane,
    },
    /// One terminal, across its incarnations.
    Terminal {
        /// The terminal.
        id: F::Terminal,
    },
    /// One shared document.
    Document {
        /// The document.
        name: F::Document,
    },
    /// One plugin.
    Plugin {
        /// The plugin.
        id: F::Plugin,
    },
}

/// A resource on one machine.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound = "")]
pub struct ResourceScope<F: IdentityFamily> {
    machine: F::Machine,
    resource: ScopeResource<F>,
}

impl<F: IdentityFamily> ResourceScope<F> {
    /// Scopes `resource` on `machine`.
    #[must_use]
    pub const fn new(machine: F::Machine, resource: ScopeResource<F>) -> Self {
        Self { machine, resource }
    }

    fn covers(&self, target: &ResolvedTarget<F>) -> bool {
        self.machine == target.machine && self.resource_covers(&target.resource)
    }

    fn contains(&self, inner: &Self) -> bool {
        self.machine == inner.machine
            && (self.resource == ScopeResource::Machine || self.resource == inner.resource)
    }

    fn resource_covers(&self, target: &TargetResource<F>) -> bool {
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
#[serde(deny_unknown_fields, bound = "")]
pub struct Grant<F: IdentityFamily> {
    permission: Permission,
    scope: ResourceScope<F>,
}

impl<F: IdentityFamily> Grant<F> {
    /// Grants `permission` over `scope`.
    #[must_use]
    pub const fn new(permission: Permission, scope: ResourceScope<F>) -> Self {
        Self { permission, scope }
    }

    pub(super) fn allows(&self, permission: Permission, target: &ResolvedTarget<F>) -> bool {
        self.permission == permission && self.scope.covers(target)
    }

    pub(super) fn includes(&self, inner: &Self) -> bool {
        self.permission == inner.permission && self.scope.contains(&inner.scope)
    }

    pub(super) const fn machine(&self) -> &F::Machine {
        &self.scope.machine
    }

    pub(super) const fn permission(&self) -> Permission {
        self.permission
    }
}

/// Where a terminal sits in the layout.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Placement<F: IdentityFamily> {
    workspace: F::Workspace,
    tab: F::Tab,
    pane: F::Pane,
}

impl<F: IdentityFamily> Placement<F> {
    /// A terminal placed in `pane` of `tab` in `workspace`.
    #[must_use]
    pub const fn new(workspace: F::Workspace, tab: F::Tab, pane: F::Pane) -> Self {
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
pub enum TargetResource<F: IdentityFamily> {
    /// The machine itself.
    Machine,
    /// A workspace.
    Workspace {
        /// The workspace.
        workspace: F::Workspace,
    },
    /// One incarnation of a terminal.
    Terminal {
        /// Where the terminal sits; `None` for a terminal outside the layout.
        placement: Option<Placement<F>>,
        /// The terminal.
        terminal: F::Terminal,
        /// The incarnation the request resolved to.
        incarnation: F::Incarnation,
    },
    /// A shared document.
    Document {
        /// The document.
        name: F::Document,
    },
    /// A plugin.
    Plugin {
        /// The plugin.
        plugin: F::Plugin,
    },
}

impl<F: IdentityFamily> TargetResource<F> {
    const fn placement(&self) -> Option<&Placement<F>> {
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
pub struct ResolvedTarget<F: IdentityFamily> {
    machine: F::Machine,
    resource: TargetResource<F>,
}

impl<F: IdentityFamily> ResolvedTarget<F> {
    /// The resolved `resource` on `machine`.
    #[must_use]
    pub const fn new(machine: F::Machine, resource: TargetResource<F>) -> Self {
        Self { machine, resource }
    }

    /// The machine that owns the resource.
    #[must_use]
    pub const fn machine(&self) -> &F::Machine {
        &self.machine
    }

    /// The resolved resource, with the identities the resolver issued.
    #[must_use]
    pub const fn resource(&self) -> &TargetResource<F> {
        &self.resource
    }
}

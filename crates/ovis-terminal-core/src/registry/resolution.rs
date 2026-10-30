//! Explicit resource lookup by typed ID, requested binding or display name.

use serde::{Deserialize, Serialize};

use crate::registry::{IdentityError, MachineId, ResourceName, RuntimeId, TerminalId};

/// Stable machine/runtime scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MachineRuntime {
    machine: MachineId,
    runtime: RuntimeId,
}

/// A terminal identity scoped by both machine and runtime lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ResourceTargetKey {
    scope: MachineRuntime,
    terminal: TerminalId,
}

impl ResourceTargetKey {
    /// Creates one fully scoped target identity.
    pub const fn new(scope: MachineRuntime, terminal: TerminalId) -> Self {
        Self { scope, terminal }
    }

    /// Machine/runtime scope.
    pub const fn scope(self) -> MachineRuntime {
        self.scope
    }
}

impl MachineRuntime {
    /// Creates the scope of a machine runtime lifetime.
    pub const fn new(machine: MachineId, runtime: RuntimeId) -> Self {
        Self { machine, runtime }
    }

    /// Machine identity.
    pub const fn machine_id(self) -> MachineId {
        self.machine
    }

    /// Runtime lifetime identity.
    pub const fn runtime_id(self) -> RuntimeId {
        self.runtime
    }
}

/// A terminal address that is independent of focus, name and layout path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceTarget {
    scope: MachineRuntime,
    terminal: TerminalId,
    name: ResourceName,
}

impl ResourceTarget {
    /// Creates a target from its immutable identity and mutable display name.
    pub const fn new(scope: MachineRuntime, terminal: TerminalId, name: ResourceName) -> Self {
        Self {
            scope,
            terminal,
            name,
        }
    }

    /// Machine identity.
    pub const fn machine_id(&self) -> MachineId {
        self.scope.machine_id()
    }

    /// Runtime lifetime identity.
    pub const fn runtime_id(&self) -> RuntimeId {
        self.scope.runtime_id()
    }

    /// Opaque terminal identity.
    pub const fn terminal_id(&self) -> TerminalId {
        self.terminal
    }

    /// Fully scoped, immutable identity key.
    pub const fn key(&self) -> ResourceTargetKey {
        ResourceTargetKey::new(self.scope, self.terminal)
    }

    /// Mutable display name.
    pub const fn name(&self) -> &ResourceName {
        &self.name
    }

    /// Renames this target without changing its identity.
    #[must_use]
    pub const fn with_name(&self, name: ResourceName) -> Self {
        Self {
            scope: self.scope,
            terminal: self.terminal,
            name,
        }
    }
}

/// One explicitly requested way to identify a target.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TargetRequest {
    /// Opaque resource identity.
    Id(ResourceTargetKey),
    /// Display name, optionally narrowed by stable machine identity.
    Name {
        /// Stable machine selector; `None` intentionally allows ambiguity.
        machine: Option<MachineId>,
        /// Display name to resolve.
        name: ResourceName,
    },
    /// A caller binding that the caller explicitly asked the registry to use.
    CallerBinding {
        /// The caller's previously established immutable binding.
        target: ResourceTargetKey,
        /// Runtime lifetime expected by this call.
        expected_scope: MachineRuntime,
    },
    /// No target was supplied; focus is never used as a fallback.
    Missing,
}

/// Stable target resolution failures with structured ambiguity candidates.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TargetResolutionError {
    /// A target-sensitive call omitted its target.
    #[error("the request did not identify a target")]
    NeedsTarget,
    /// The requested name resolves to multiple resources.
    #[error("the requested name identifies multiple resources")]
    AmbiguousTarget {
        /// Every matching resource, with stable machine/runtime/resource identities.
        candidates: Vec<ResourceTarget>,
    },
    /// No live resource matches the requested identity or name.
    #[error("no resource matches the requested target")]
    NotFound,
    /// A previously bound resource no longer exists in the registry.
    #[error("the caller's bound resource is no longer present")]
    StaleTarget,
    /// The caller's binding belongs to a different machine.
    #[error("the caller's binding belongs to a different machine")]
    CrossMachine,
    /// A typed resource belongs to another runtime lifetime.
    #[error(transparent)]
    Identity(#[from] IdentityError),
}

impl TargetResolutionError {
    /// Stable machine-readable error kind.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NeedsTarget => "needs_target",
            Self::AmbiguousTarget { .. } => "ambiguous_target",
            Self::NotFound => "not_found",
            Self::StaleTarget => "stale_target",
            Self::CrossMachine => "cross_machine",
            Self::Identity(error) => error.kind(),
        }
    }

    /// Recovery guidance for the error kind.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::NeedsTarget => "supply a resource ID or explicitly request a caller binding",
            Self::AmbiguousTarget { .. } => {
                "select one candidate by its machine and opaque resource IDs"
            }
            Self::NotFound => "refresh the resource roster and supply a current target",
            Self::StaleTarget => {
                "refresh the caller binding; the request was not redirected to a replacement"
            }
            Self::CrossMachine => {
                "refresh the caller binding using the requested stable machine identity"
            }
            Self::Identity(error) => error.hint(),
        }
    }

    /// Ambiguity candidates, if the failure requires a caller choice.
    pub fn candidates(&self) -> Option<&[ResourceTarget]> {
        match self {
            Self::AmbiguousTarget { candidates } => Some(candidates),
            Self::NeedsTarget
            | Self::NotFound
            | Self::StaleTarget
            | Self::CrossMachine
            | Self::Identity(_) => None,
        }
    }
}

/// A read-only target index; it contains no focused or layout selection.
#[derive(Debug, Clone, Default)]
pub struct TargetIndex {
    targets: Vec<ResourceTarget>,
}

impl TargetIndex {
    /// Builds an index from the currently visible resources.
    pub fn from_targets(targets: impl IntoIterator<Item = ResourceTarget>) -> Self {
        Self {
            targets: targets.into_iter().collect(),
        }
    }

    /// Resolves exactly the requested identity or binding.
    ///
    /// # Errors
    ///
    /// Returns `needs_target`, `ambiguous_target`, `not_found`, `stale_target` or
    /// `cross_lifetime`; no failure falls back to focus or another resource.
    pub fn resolve(&self, request: TargetRequest) -> Result<ResourceTarget, TargetResolutionError> {
        match request {
            TargetRequest::Id(id) => self
                .targets
                .iter()
                .find(|target| target.key() == id)
                .cloned()
                .ok_or(TargetResolutionError::NotFound),
            TargetRequest::Name { machine, name } => {
                let mut candidates = self
                    .targets
                    .iter()
                    .filter(|target| target.name() == &name)
                    .filter(|target| {
                        machine.is_none_or(|machine_id| target.machine_id() == machine_id)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                candidates.sort_by_key(ResourceTarget::key);
                match candidates.len() {
                    0 => Err(TargetResolutionError::NotFound),
                    1 => Ok(candidates.remove(0)),
                    _ => Err(TargetResolutionError::AmbiguousTarget { candidates }),
                }
            }
            TargetRequest::CallerBinding {
                target,
                expected_scope,
            } => {
                if target.scope().machine_id() != expected_scope.machine_id() {
                    return Err(TargetResolutionError::CrossMachine);
                }
                if target.scope().runtime_id() != expected_scope.runtime_id() {
                    return Err(IdentityError::CrossLifetime {
                        expected: expected_scope.runtime_id(),
                        actual: target.scope().runtime_id(),
                    }
                    .into());
                }
                self.targets
                    .iter()
                    .find(|candidate| candidate.key() == target)
                    .cloned()
                    .ok_or(TargetResolutionError::StaleTarget)
            }
            TargetRequest::Missing => Err(TargetResolutionError::NeedsTarget),
        }
    }
}

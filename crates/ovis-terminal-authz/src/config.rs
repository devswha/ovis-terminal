//! What an authority is bound to, its owner credential and its table bounds.

use ovis_terminal_protocol::schema::ids::{MachineId, RuntimeLifetimeId};
use ovis_terminal_protocol::schema::scalar::TimestampMs;

use crate::identity::PrincipalId;

/// The machine and runtime lifetime whose credentials an authority issues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeBinding {
    pub(crate) machine: MachineId,
    pub(crate) lifetime: RuntimeLifetimeId,
}

impl RuntimeBinding {
    /// Binds credentials to `lifetime` on `machine`.
    #[must_use]
    pub const fn new(machine: MachineId, lifetime: RuntimeLifetimeId) -> Self {
        Self { machine, lifetime }
    }
}

/// The principal and expiry of the bootstrap owner credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerSpec {
    pub(crate) principal: PrincipalId,
    pub(crate) expires_at: TimestampMs,
}

impl OwnerSpec {
    /// The owner `principal`, valid until `expires_at`.
    #[must_use]
    pub const fn new(principal: PrincipalId, expires_at: TimestampMs) -> Self {
        Self {
            principal,
            expires_at,
        }
    }
}

/// Bounds on the credential table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorityLimits {
    pub(crate) max_credentials: usize,
    pub(crate) max_grants: usize,
}

impl AuthorityLimits {
    /// 1024 credentials of at most 64 grants each.
    pub const DEFAULT: Self = Self {
        max_credentials: 1024,
        max_grants: 64,
    };

    /// Caps the credential table, the owner credential included.
    #[must_use]
    pub const fn with_max_credentials(mut self, credentials: usize) -> Self {
        self.max_credentials = credentials;
        self
    }

    /// Caps the grants of one credential.
    #[must_use]
    pub const fn with_max_grants(mut self, grants: usize) -> Self {
        self.max_grants = grants;
        self
    }
}

/// Everything [`crate::Authority::bootstrap`] needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityConfig {
    pub(crate) binding: RuntimeBinding,
    pub(crate) owner: OwnerSpec,
    pub(crate) limits: AuthorityLimits,
}

impl AuthorityConfig {
    /// An authority for `binding` whose owner is `owner`, with default limits.
    #[must_use]
    pub const fn new(binding: RuntimeBinding, owner: OwnerSpec) -> Self {
        Self {
            binding,
            owner,
            limits: AuthorityLimits::DEFAULT,
        }
    }

    /// Replaces the table bounds.
    #[must_use]
    pub const fn with_limits(mut self, limits: AuthorityLimits) -> Self {
        self.limits = limits;
        self
    }
}

//! What an authority is bound to, its owner credential and its table bounds.

use ovis_terminal_protocol::schema::scalar::TimestampMs;

use crate::family::IdentityFamily;
use crate::identity::PrincipalId;

/// The machine and runtime lifetime whose credentials an authority issues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeBinding<F: IdentityFamily> {
    pub(crate) machine: F::Machine,
    pub(crate) lifetime: F::Lifetime,
}

impl<F: IdentityFamily> RuntimeBinding<F> {
    /// Binds credentials to `lifetime` on `machine`.
    #[must_use]
    pub const fn new(machine: F::Machine, lifetime: F::Lifetime) -> Self {
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
    pub(crate) credentials: usize,
    pub(crate) grants: usize,
    pub(crate) tombstones: usize,
}

impl AuthorityLimits {
    /// 1024 credentials of at most 64 grants each, and 1024 remembered revocations.
    pub const DEFAULT: Self = Self {
        credentials: 1024,
        grants: 64,
        tombstones: 1024,
    };

    /// Caps the credential table, the owner credential included.
    #[must_use]
    pub const fn with_max_credentials(mut self, credentials: usize) -> Self {
        self.credentials = credentials;
        self
    }

    /// Caps the revocations remembered after their credential records are reclaimed.
    #[must_use]
    pub const fn with_max_tombstones(mut self, tombstones: usize) -> Self {
        self.tombstones = tombstones;
        self
    }

    /// Caps the grants of one credential.
    #[must_use]
    pub const fn with_max_grants(mut self, grants: usize) -> Self {
        self.grants = grants;
        self
    }
}

/// Everything [`crate::Authority::bootstrap`] needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityConfig<F: IdentityFamily> {
    pub(crate) binding: RuntimeBinding<F>,
    pub(crate) owner: OwnerSpec,
    pub(crate) limits: AuthorityLimits,
}

impl<F: IdentityFamily> AuthorityConfig<F> {
    /// An authority for `binding` whose owner is `owner`, with default limits.
    #[must_use]
    pub const fn new(binding: RuntimeBinding<F>, owner: OwnerSpec) -> Self {
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

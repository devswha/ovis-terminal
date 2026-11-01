//! What a rotation asks for and what a rotation or withdrawal returns.

use ovis_terminal_protocol::schema::scalar::TimestampMs;

use super::PrincipalEpoch;
use crate::authority::IssuedCredential;
use crate::family::IdentityFamily;
use crate::identity::{CredentialEpoch, CredentialId, PrincipalId};

/// A request to replace one credential, made at one instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rotation {
    pub(crate) credential: CredentialId,
    pub(crate) expected: PrincipalEpoch,
    pub(crate) expires_at: Option<TimestampMs>,
    pub(crate) at: TimestampMs,
}

impl Rotation {
    /// Replaces `credential` at `at`, provided its principal is still at `expected`.
    #[must_use]
    pub const fn new(credential: CredentialId, expected: PrincipalEpoch, at: TimestampMs) -> Self {
        Self {
            credential,
            expected,
            expires_at: None,
            at,
        }
    }

    /// The replacement expires at `expires_at` instead of the replaced credential's expiry.
    #[must_use]
    pub const fn until(mut self, expires_at: TimestampMs) -> Self {
        self.expires_at = Some(expires_at);
        self
    }
}

/// A credential that no longer authenticates, and where its principal stands now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Withdrawn {
    pub(crate) credential: CredentialId,
    pub(crate) epoch: CredentialEpoch,
    pub(crate) principal: PrincipalId,
    pub(crate) principal_epoch: PrincipalEpoch,
}

impl Withdrawn {
    /// The withdrawn credential.
    #[must_use]
    pub const fn credential(&self) -> CredentialId {
        self.credential
    }

    /// The credential's epoch after withdrawal.
    #[must_use]
    pub const fn epoch(&self) -> CredentialEpoch {
        self.epoch
    }

    /// The principal it authenticated.
    #[must_use]
    pub const fn principal(&self) -> &PrincipalId {
        &self.principal
    }

    /// The principal's epoch after the withdrawal.
    #[must_use]
    pub const fn principal_epoch(&self) -> PrincipalEpoch {
        self.principal_epoch
    }
}

/// The replacement a rotation issued and the credential it withdrew in the same step.
#[derive(Debug)]
pub struct Rotated<F: IdentityFamily> {
    pub(crate) issued: IssuedCredential<F>,
    pub(crate) withdrawn: Withdrawn,
}

impl<F: IdentityFamily> Rotated<F> {
    /// The replacement, with its bearer text.
    #[must_use]
    pub const fn issued(&self) -> &IssuedCredential<F> {
        &self.issued
    }

    /// The replaced credential.
    #[must_use]
    pub const fn withdrawn(&self) -> &Withdrawn {
        &self.withdrawn
    }
}

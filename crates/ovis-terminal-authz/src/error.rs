//! The authority boundary error, its stable kinds and recovery hints.

use std::fmt;

use ovis_terminal_protocol::schema::scalar::TimestampMs;
use thiserror::Error;

use crate::identity::{CredentialEpoch, CredentialId, PrincipalId};
use crate::permission::{Action, Permission};
use crate::rotation::PrincipalEpoch;

/// Why an authentication, authorization, issuance or endpoint check refused.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AuthzError {
    /// The connection presented no credential.
    #[error("no credential was presented")]
    Unauthenticated,
    /// The presented bytes do not name a credential this authority issued.
    #[error("the presented credential does not match an issued credential")]
    InvalidCredential,
    /// The credential reached its expiry.
    #[error("credential {credential} stopped being valid at {} ms", .expired_at.get())]
    CredentialExpired {
        /// The expired credential.
        credential: CredentialId,
        /// When it expired.
        expired_at: TimestampMs,
    },
    /// The credential was revoked after the session or decision was made.
    #[error("credential {credential} was withdrawn at epoch {}", .epoch.get())]
    CredentialRevoked {
        /// The withdrawn credential.
        credential: CredentialId,
        /// Its current epoch.
        epoch: CredentialEpoch,
        /// The current authority epoch of its principal.
        principal_epoch: PrincipalEpoch,
    },
    /// A rotation named a principal epoch that another change already moved past.
    #[error("the principal's authority is at epoch {}, not the one the rotation named", .principal_epoch.get())]
    PrincipalEpochConflict {
        /// The principal's current epoch.
        principal_epoch: PrincipalEpoch,
    },
    /// No grant of the principal covers the action on the target.
    #[error("principal {principal} holds no {required} grant covering {action} on this target")]
    Forbidden {
        /// The refused principal.
        principal: PrincipalId,
        /// The requested action.
        action: Action,
        /// The permission the action needs.
        required: Permission,
    },
    /// An issuance asked for more than the issuer holds.
    #[error("the requested grant exceeds the issuer's own authority: {exceeded}")]
    GrantExceedsIssuer {
        /// What the request exceeded.
        exceeded: Exceeded,
    },
    /// A decision was presented for a commit it was not made for.
    #[error("the decision does not cover this commit: {reason}")]
    DecisionMismatch {
        /// Which binding differs.
        reason: MismatchReason,
    },
    /// The serving endpoint itself is reachable by other accounts.
    #[error("the endpoint is not private to its owner: {defect}")]
    EndpointInsecure {
        /// The detected exposure.
        defect: EndpointDefect,
    },
    /// The connecting peer is not the endpoint owner.
    #[error("the connecting peer is not the endpoint owner")]
    PeerRejected,
    /// A request document violates its closed schema.
    #[error("request field {field_path} is malformed")]
    InvalidParams {
        /// Path of the offending field.
        field_path: String,
        /// The decoder failure.
        #[source]
        source: serde_json::Error,
    },
    /// A request field parsed but holds an unacceptable value.
    #[error("request field {field_path} holds an unacceptable value")]
    InvalidValue {
        /// Path of the offending field.
        field_path: &'static str,
    },
    /// The named credential does not exist.
    #[error("credential {credential} does not exist")]
    CredentialNotFound {
        /// The missing credential.
        credential: CredentialId,
    },
    /// A bounded table is full.
    #[error("the {limit} limit is reached")]
    LimitExceeded {
        /// The exhausted bound.
        limit: Limit,
    },
    /// A thread panicked while holding the credential table.
    #[error("the credential table is unusable after a panic")]
    Poisoned,
}

impl AuthzError {
    /// The stable machine-readable kind clients branch on.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Unauthenticated => "unauthenticated",
            Self::InvalidCredential => "invalid_credential",
            Self::CredentialExpired { .. } => "credential_expired",
            Self::CredentialRevoked { .. } => "credential_revoked",
            Self::PrincipalEpochConflict { .. } => "version_conflict",
            Self::Forbidden { .. } => "forbidden",
            Self::GrantExceedsIssuer { .. } => "grant_exceeds_issuer",
            Self::DecisionMismatch { .. } => "decision_mismatch",
            Self::EndpointInsecure { .. } => "endpoint_insecure",
            Self::PeerRejected => "peer_rejected",
            Self::InvalidParams { .. } | Self::InvalidValue { .. } => "invalid_params",
            Self::CredentialNotFound { .. } => "not_found",
            Self::LimitExceeded { .. } => "limit_exceeded",
            Self::Poisoned => "internal",
        }
    }

    /// Recovery guidance published with the kind.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Unauthenticated => "present a credential in hello",
            Self::InvalidCredential => "obtain a credential issued by this runtime lifetime",
            Self::CredentialExpired { .. } => "request a new credential from an administrator",
            Self::CredentialRevoked { .. } => {
                "re-authenticate with a current credential; queued work was not written"
            }
            Self::PrincipalEpochConflict { .. } => {
                "read the principal's current epoch and decide again whether to rotate"
            }
            Self::Forbidden { .. } => "ask an administrator for a grant covering this target",
            Self::GrantExceedsIssuer { .. } => {
                "request only permissions, scopes and expiry you hold"
            }
            Self::DecisionMismatch { .. } => "authorize the operation against its commit target",
            Self::EndpointInsecure { .. } => "restrict the endpoint to its owning account",
            Self::PeerRejected => "connect as the account that owns the runtime",
            Self::InvalidParams { .. } | Self::InvalidValue { .. } => {
                "correct the field named by field_path"
            }
            Self::CredentialNotFound { .. } => "list credentials to find a live id",
            Self::LimitExceeded { .. } => "revoke unused credentials or request fewer grants",
            Self::Poisoned => "restart the control server",
        }
    }

    /// The principal's current epoch, when the failure is about its authority moving.
    #[must_use]
    pub const fn principal_epoch(&self) -> Option<PrincipalEpoch> {
        match self {
            Self::CredentialRevoked {
                principal_epoch, ..
            }
            | Self::PrincipalEpochConflict { principal_epoch } => Some(*principal_epoch),
            _ => None,
        }
    }

    /// The offending field of a malformed request, when the failure names one.
    #[must_use]
    pub fn field_path(&self) -> Option<&str> {
        match self {
            Self::InvalidParams { field_path, .. } => Some(field_path),
            Self::InvalidValue { field_path } => Some(field_path),
            Self::Unauthenticated
            | Self::InvalidCredential
            | Self::CredentialExpired { .. }
            | Self::CredentialRevoked { .. }
            | Self::PrincipalEpochConflict { .. }
            | Self::Forbidden { .. }
            | Self::GrantExceedsIssuer { .. }
            | Self::DecisionMismatch { .. }
            | Self::EndpointInsecure { .. }
            | Self::PeerRejected
            | Self::CredentialNotFound { .. }
            | Self::LimitExceeded { .. }
            | Self::Poisoned => None,
        }
    }
}

/// What part of an issuance exceeded the issuer's authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Exceeded {
    /// The issuer holds no covering grant of this permission.
    Permission(Permission),
    /// The grant names a machine other than this authority's.
    Machine,
    /// The credential would outlive the issuer's own credential.
    Expiry,
}

/// Which binding of a decision differs from the commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MismatchReason {
    /// The decision came from another authority instance.
    Authority,
    /// The commit performs a different action.
    Action,
    /// The commit lands on a different resource or incarnation.
    Target,
}

/// How a serving endpoint is exposed beyond its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EndpointDefect {
    /// The socket or its directory belongs to another account.
    ForeignOwner,
    /// Group or other permission bits are set.
    SharedMode,
    /// The pipe has a null DACL, granting everyone access.
    NullDacl,
    /// The pipe DACL allows an account other than the owner.
    ForeignTrustee,
    /// The pipe accepts clients from other machines.
    RemoteClients,
    /// An additional pipe instance was opened while the name was unclaimed.
    UnclaimedName,
    /// A first pipe instance was opened while the name was already claimed.
    NameAlreadyClaimed,
    /// More instances were closed than were opened.
    InstanceUnderflow,
}

/// A bounded table of the authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Limit {
    /// Live credentials per authority.
    Credentials,
    /// Grants per credential.
    Grants,
}

impl fmt::Display for Exceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Permission(permission) => write!(f, "permission {permission} is not held"),
            Self::Machine => f.write_str("another machine is named"),
            Self::Expiry => f.write_str("expiry is later than the issuer's"),
        }
    }
}

impl fmt::Display for MismatchReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Authority => "issued by another authority",
            Self::Action => "different action",
            Self::Target => "different target",
        })
    }
}

impl fmt::Display for EndpointDefect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ForeignOwner => "owned by another account",
            Self::SharedMode => "group or other access bits set",
            Self::NullDacl => "null DACL",
            Self::ForeignTrustee => "DACL allows another account",
            Self::RemoteClients => "remote clients accepted",
            Self::UnclaimedName => "additional instance without a first instance",
            Self::NameAlreadyClaimed => "first instance while the name is claimed",
            Self::InstanceUnderflow => "closed more instances than opened",
        })
    }
}

impl fmt::Display for Limit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Credentials => "credential",
            Self::Grants => "grant",
        })
    }
}

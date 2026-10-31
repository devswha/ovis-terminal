//! The lease boundary error, its stable kinds and recovery hints.

use ovis_terminal_authz::AuthzError;

use super::Lease;
use crate::registry::{IdentityError, LeaseGeneration, LeaseId};

/// Why a lease operation was refused; nothing changed when this is returned.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LeaseError {
    /// The credential does not grant the action this operation needs.
    #[error("the authority refused this lease operation")]
    Authority(#[source] AuthzError),
    /// Another owner holds the slot.
    #[error(
        "{} is held by client {} of {} at epoch {}",
        holder.lease_id,
        holder.owner.client.as_str(),
        holder.owner.principal,
        holder.epoch.get()
    )]
    Conflict {
        /// Current holder.
        holder: Box<Lease>,
    },
    /// The takeover named an epoch the slot no longer has.
    #[error("takeover expected epoch {} but {} holds epoch {}", expected.get(), current.lease_id, current.epoch.get())]
    EpochMismatch {
        /// Epoch the caller presented.
        expected: LeaseGeneration,
        /// Lease holding the slot now.
        current: Box<Lease>,
    },
    /// The caller does not hold the lease this operation requires.
    #[error("the caller holds no current lease for this operation")]
    NoLease,
    /// The lease a queued operation was admitted under no longer holds its slot.
    #[error("{lease} stopped holding its slot after admission")]
    Revoked {
        /// Lease the operation was admitted under.
        lease: LeaseId,
    },
    /// The slot has no owner to displace.
    #[error("the slot has no owner to take over")]
    NothingToTake,
    /// The caller already owns the slot.
    #[error("the caller already holds {}", held.lease_id)]
    SelfLease {
        /// The caller's lease.
        held: Box<Lease>,
    },
    /// Another lease owns geometry.
    #[error("geometry belongs to {}", owner.lease_id)]
    NotGeometryOwner {
        /// Current geometry lease.
        owner: Box<Lease>,
    },
    /// An observe lease was presented for a controlling operation.
    #[error("an observe lease cannot change the terminal")]
    ObserveOnly,
    /// The resolved target is not one terminal incarnation.
    #[error("leases exist only on a terminal target")]
    NotTerminal,
    /// A request field holds a value no lease can have.
    #[error("request field {field_path} holds an unacceptable value")]
    InvalidValue {
        /// Offending field.
        field_path: &'static str,
    },
    /// The slot epoch cannot advance further.
    #[error("the lease epoch cannot advance")]
    Exhausted(#[source] IdentityError),
}

impl LeaseError {
    /// The stable machine-readable kind clients branch on.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Authority(source) => source.kind(),
            Self::Conflict { .. } => "lease_conflict",
            Self::EpochMismatch { .. } => "lease_epoch_mismatch",
            Self::NoLease => "no_lease",
            Self::Revoked { .. } => "lease_revoked",
            Self::NothingToTake => "no_lease_to_take",
            Self::SelfLease { .. } => "self_lease",
            Self::NotGeometryOwner { .. } => "not_geometry_owner",
            Self::ObserveOnly => "observe_forbidden_op",
            Self::NotTerminal | Self::InvalidValue { .. } => "invalid_params",
            Self::Exhausted(_) => "limit_exceeded",
        }
    }

    /// Recovery guidance published with the kind.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Authority(source) => source.hint(),
            Self::Conflict { .. } => {
                "attach as an observer or take over explicitly at the holder's epoch"
            }
            Self::EpochMismatch { .. } => {
                "read the current lease and retry the takeover at its epoch"
            }
            Self::NoLease => "acquire the interact lease or present the one you hold",
            Self::Revoked { .. } => "read the current lease before resending",
            Self::NothingToTake => "acquire the slot instead of taking it over",
            Self::SelfLease { .. } => "keep using the lease you already hold",
            Self::NotGeometryOwner { .. } => {
                "request geometry and wait for the owner to release it"
            }
            Self::ObserveOnly => "attach with an interact grant to change the terminal",
            Self::NotTerminal => "target exactly one terminal",
            Self::InvalidValue { .. } => "send the epoch the current lease reports",
            Self::Exhausted(_) => "recover the terminal to start a new incarnation",
        }
    }
}

//! Lease identities, owners and the records every lease event carries.

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_protocol::schema::ids::ClientId;
use serde::{Deserialize, Serialize};

use crate::registry::{LeaseGeneration, LeaseId};

/// Authority a lease grants; interact and geometry are independent slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LeaseSlot {
    /// Writes input to the terminal.
    Interact,
    /// Sets the terminal grid size.
    Geometry,
    /// Reads only; any number coexist and none changes input or the grid.
    Observe,
}

/// A slot with exactly one owner at a time, acquired or taken over explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ControlSlot {
    /// The input lease.
    Interact,
    /// The grid-size lease.
    Geometry,
}

impl From<ControlSlot> for LeaseSlot {
    fn from(slot: ControlSlot) -> Self {
        match slot {
            ControlSlot::Interact => Self::Interact,
            ControlSlot::Geometry => Self::Geometry,
        }
    }
}

/// The authenticated principal together with the client label it attached as.
///
/// Both must match for a lease to act; knowing a lease id grants nothing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Owner {
    /// Principal the credential authenticated.
    pub principal: PrincipalId,
    /// Client label of the attachment.
    pub client: ClientId,
}

impl Owner {
    /// Names the owner of a lease.
    pub const fn new(principal: PrincipalId, client: ClientId) -> Self {
        Self { principal, client }
    }
}

/// One lease: identity, slot, ownership epoch and owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Lease {
    /// Lease identity, never reused.
    pub lease_id: LeaseId,
    /// Slot the lease holds.
    pub slot: LeaseSlot,
    /// Epoch of the slot when this lease was granted.
    pub epoch: LeaseGeneration,
    /// Who holds it.
    pub owner: Owner,
}

/// An owner asking for a lease under an identity reserved for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Claim {
    pub(super) owner: Owner,
    pub(super) fresh: LeaseId,
}

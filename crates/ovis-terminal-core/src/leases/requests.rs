//! Typed lease requests, each bound to an authenticated caller.

use ovis_terminal_authz::{Action, Session};
use ovis_terminal_protocol::schema::ids::ClientId;
use ovis_terminal_protocol::schema::methods::attach::TakeoverRequest;
use ovis_terminal_protocol::schema::scalar::TimestampMs;

use super::LeaseError;
use super::lease::{Claim, ControlSlot, Owner};
use crate::registry::{LeaseGeneration, LeaseId};

/// One authenticated session acting as one client label at one instant.
#[derive(Debug, Clone, Copy)]
pub struct Caller<'a> {
    pub(super) session: &'a Session,
    client: &'a ClientId,
    pub(super) at: TimestampMs,
}

impl<'a> Caller<'a> {
    /// Binds a request to its session, client label and request time.
    pub const fn new(session: &'a Session, client: &'a ClientId, at: TimestampMs) -> Self {
        Self {
            session,
            client,
            at,
        }
    }

    pub(super) fn owner(&self) -> Owner {
        Owner::new(self.session.principal().clone(), self.client.clone())
    }

    pub(super) fn claim(&self, fresh: LeaseId) -> Claim {
        Claim {
            owner: self.owner(),
            fresh,
        }
    }
}

/// Acquisition of a control slot under a durably reserved lease identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Acquire {
    /// Slot requested.
    pub slot: ControlSlot,
    /// Identity the lease receives if granted now or later by transfer.
    pub fresh: LeaseId,
}

impl Acquire {
    /// Requests `slot` under `fresh`.
    pub const fn new(slot: ControlSlot, fresh: LeaseId) -> Self {
        Self { slot, fresh }
    }
}

/// The write path a request uses; each needs its own grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WriteKind {
    /// Text, keys or paste through the ordered input path.
    Input,
    /// Raw bytes that bypass prompt protection.
    Raw,
}

impl WriteKind {
    pub(super) const fn action(self) -> Action {
        match self {
            Self::Input => Action::Input,
            Self::Raw => Action::RawInput,
        }
    }
}

/// A write and the interact lease the writer presents, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Write {
    /// Write path.
    pub kind: WriteKind,
    /// Presented interact lease.
    pub lease: Option<LeaseId>,
}

impl Write {
    /// Describes a write presenting `lease`.
    pub const fn new(kind: WriteKind, lease: Option<LeaseId>) -> Self {
        Self { kind, lease }
    }
}

/// An explicit takeover bound to the epoch the caller observed; never implied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Takeover {
    slot: ControlSlot,
    expected: LeaseGeneration,
    fresh: LeaseId,
}

impl Takeover {
    /// Parses the wire request for `slot` once; `fresh` becomes the new lease identity.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` when the expected epoch is zero, which no lease ever holds.
    pub fn new(
        slot: ControlSlot,
        request: &TakeoverRequest,
        fresh: LeaseId,
    ) -> Result<Self, LeaseError> {
        let expected = LeaseGeneration::new(request.lease_epoch.get()).map_err(|_| {
            LeaseError::InvalidValue {
                field_path: "takeover.lease_epoch",
            }
        })?;
        Ok(Self {
            slot,
            expected,
            fresh,
        })
    }

    /// Slot to take over.
    pub const fn slot(&self) -> ControlSlot {
        self.slot
    }

    /// Epoch the caller expects the current lease to hold.
    pub const fn expected(&self) -> LeaseGeneration {
        self.expected
    }

    /// Identity of the lease the caller receives.
    pub const fn fresh(&self) -> LeaseId {
        self.fresh
    }
}

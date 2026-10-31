//! Ordered lease events and the durable unit that commits them together.

use ovis_terminal_store::OpId;
use serde::{Deserialize, Serialize};

use super::Lease;
use crate::events::{Commit, EventData, ResourceId};
use crate::registry::TerminalId;

/// Why a lease stopped granting authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RevokeReason {
    /// The owner released it.
    Released,
    /// The owner's connection ended.
    Disconnected,
    /// An authorized takeover displaced it.
    TakenOver,
}

/// One lease transition, published only after its durable commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum LeaseEvent {
    /// An interact or observe lease was granted.
    Acquired {
        /// Granted lease.
        lease: Lease,
        /// Lease an explicit takeover displaced.
        displaced: Option<Lease>,
    },
    /// Geometry ownership moved to `lease`.
    Geometry {
        /// New geometry lease.
        lease: Lease,
        /// Geometry lease it replaced by takeover or transfer.
        previous: Option<Lease>,
    },
    /// A lease stopped granting authority.
    Revoked {
        /// Revoked lease.
        lease: Lease,
        /// Cause.
        reason: RevokeReason,
        /// New owner, for a takeover.
        displaced_by: Option<Lease>,
    },
}

impl EventData for LeaseEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Acquired { .. } => "lease.acquired",
            Self::Geometry { .. } => "lease.geometry",
            Self::Revoked { .. } => "lease.revoked",
        }
    }
}

/// The events one lease operation produced, in the order they must commit.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Transition {
    /// Terminal whose leases changed.
    pub terminal: TerminalId,
    /// Events in commit order; empty when nothing changed.
    pub events: Vec<LeaseEvent>,
}

impl Transition {
    pub(super) const fn new(terminal: TerminalId, events: Vec<LeaseEvent>) -> Self {
        Self { terminal, events }
    }

    /// One durable unit committing every event under `op`, or `None` when nothing changed.
    pub fn into_commit(self, op: OpId) -> Option<Commit<LeaseEvent>> {
        let affected = vec![ResourceId::Terminal(self.terminal)];
        let mut events = self.events.into_iter();
        let first = events.next()?;
        let commit = Commit::new(op, first, affected.clone());
        Some(events.fold(commit, |commit, event| {
            commit.event(event, affected.clone())
        }))
    }
}

/// A lease the caller now holds and the transition that granted it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Granted {
    /// The caller's lease.
    pub lease: Lease,
    /// Events to commit; empty when the caller already held this lease.
    pub transition: Transition,
}

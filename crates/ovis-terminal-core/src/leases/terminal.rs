//! Lease operations of one terminal incarnation, each authorized by the runtime authority.

use std::sync::Arc;

use ovis_terminal_authz::{
    AccessRequest, Action, Authority, Decision, ResolvedTarget, TargetResource,
};
use ovis_terminal_protocol::schema::scalar::TimestampMs;

use super::LeaseError;
use super::admission::LeaseView;
use super::event::{Granted, Transition};
use super::lease::{ControlSlot, Lease, Owner};
use super::requests::{Acquire, Caller, Takeover, Write};
use super::table::Table;
use crate::registry::{LeaseId, RegistryIdentities};

type Registry = RegistryIdentities;

/// Permission to write or resize under the lease it was admitted with; recheck it at the
/// write boundary so revocation and takeover fence work that is still queued.
#[derive(Debug)]
pub struct Authorization {
    action: Action,
    decision: Decision<Registry>,
    lease: Option<Lease>,
}

impl Authorization {
    /// The lease the operation was admitted under; `None` for a write on an unleased terminal.
    pub const fn lease(&self) -> Option<&Lease> {
        self.lease.as_ref()
    }
}

/// Interact, geometry and observe leases of one terminal incarnation.
///
/// Every request is authorized for its own action first, so an observe credential can never
/// acquire input or geometry and a takeover needs the separate takeover grant.
#[derive(Debug)]
pub struct TerminalLeases {
    authority: Arc<Authority<Registry>>,
    target: ResolvedTarget<Registry>,
    table: Table,
}

const fn slot_action(slot: ControlSlot) -> Action {
    match slot {
        ControlSlot::Interact => Action::Input,
        ControlSlot::Geometry => Action::Resize,
    }
}

impl TerminalLeases {
    /// Starts with no lease on `target`.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` when `target` is not a terminal.
    pub fn new(
        authority: Arc<Authority<Registry>>,
        target: ResolvedTarget<Registry>,
    ) -> Result<Self, LeaseError> {
        let TargetResource::Terminal { terminal, .. } = target.resource() else {
            return Err(LeaseError::NotTerminal);
        };
        let table = Table::new(*terminal);
        Ok(Self {
            authority,
            target,
            table,
        })
    }

    fn decide(
        &self,
        caller: &Caller<'_>,
        action: Action,
    ) -> Result<Decision<Registry>, LeaseError> {
        let request = AccessRequest::new(action, self.target.clone(), caller.at);
        self.authority
            .authorize(caller.session, &request)
            .map_err(LeaseError::Authority)
    }

    /// Attaches an observer; input and geometry owners are untouched.
    ///
    /// # Errors
    ///
    /// Returns the authority's refusal when the caller may not subscribe.
    pub fn observe(&mut self, caller: Caller<'_>, fresh: LeaseId) -> Result<Granted, LeaseError> {
        self.decide(&caller, Action::Subscribe)?;
        self.table.observe(caller.claim(fresh))
    }

    /// Acquires a free slot; a held slot is never displaced. A refused geometry request
    /// joins the hand-on queue once, keeping its first position and claimed identity.
    ///
    /// # Errors
    ///
    /// Returns `forbidden` without the slot's grant and `lease_conflict` naming the holder.
    pub fn acquire(&mut self, caller: Caller<'_>, request: Acquire) -> Result<Granted, LeaseError> {
        self.decide(&caller, slot_action(request.slot))?;
        self.table
            .acquire(request.slot, caller.claim(request.fresh))
    }

    /// Displaces the current owner when the caller holds the takeover grant and names the
    /// current epoch; the transition revokes the old lease before granting the new one.
    ///
    /// # Errors
    ///
    /// Returns `forbidden`, `no_lease_to_take`, `self_lease` or `lease_epoch_mismatch`.
    pub fn take_over(
        &mut self,
        caller: Caller<'_>,
        request: &Takeover,
    ) -> Result<Granted, LeaseError> {
        self.decide(&caller, Action::Takeover)?;
        self.decide(&caller, slot_action(request.slot()))?;
        self.table.take_over(caller.owner(), request)
    }

    /// Releases a lease the caller owns; released geometry passes to the next requester.
    ///
    /// # Errors
    ///
    /// Returns `no_lease` unless the caller owns `lease`.
    pub fn release(
        &mut self,
        caller: Caller<'_>,
        lease: LeaseId,
    ) -> Result<Transition, LeaseError> {
        self.decide(&caller, Action::Subscribe)?;
        self.table.release(&caller.owner(), lease)
    }

    /// Revokes every lease of a closed connection in slot order: interact, geometry, observe.
    ///
    /// # Errors
    ///
    /// Returns `limit_exceeded` only when the geometry epoch cannot advance for a hand-on.
    pub fn disconnect(&mut self, owner: &Owner) -> Result<Transition, LeaseError> {
        self.table.disconnect(owner)
    }

    /// Admits a write: while any lease exists only the current interact owner may write,
    /// and an unleased terminal still needs the write grant.
    ///
    /// # Errors
    ///
    /// Returns `forbidden` without the grant and `no_lease` without the current interact lease.
    pub fn authorize_write(
        &self,
        caller: Caller<'_>,
        request: Write,
    ) -> Result<Authorization, LeaseError> {
        let action = request.kind.action();
        let decision = self.decide(&caller, action)?;
        let lease = self.table.check_write(&caller.owner(), request.lease)?;
        Ok(Authorization {
            action,
            decision,
            lease,
        })
    }

    /// Admits a resize by the geometry owner.
    ///
    /// # Errors
    ///
    /// Returns `forbidden`, `observe_forbidden_op`, `no_lease` or `not_geometry_owner`.
    pub fn authorize_resize(
        &self,
        caller: Caller<'_>,
        lease: LeaseId,
    ) -> Result<Authorization, LeaseError> {
        let decision = self.decide(&caller, Action::Resize)?;
        let lease = self.table.check_resize(&caller.owner(), lease)?;
        Ok(Authorization {
            action: Action::Resize,
            decision,
            lease: Some(lease),
        })
    }

    /// Rechecks an admitted operation at its write boundary.
    ///
    /// # Errors
    ///
    /// Returns the authority's refusal for a lapsed credential, `lease_revoked` when the
    /// admitting lease moved, or `no_lease` when a lease appeared on an unleased terminal.
    pub fn recheck(&self, admitted: &Authorization, at: TimestampMs) -> Result<(), LeaseError> {
        let commit = AccessRequest::new(admitted.action, self.target.clone(), at);
        self.authority
            .recheck(&admitted.decision, &commit)
            .map_err(LeaseError::Authority)?;
        self.table.fence(admitted.lease.as_ref())
    }

    /// Current leases and the geometry hand-on queue.
    pub fn view(&self) -> LeaseView {
        self.table.view()
    }
}

//! The per-terminal lease state machine; callers authorize before reaching it.

use std::collections::VecDeque;

use super::event::{Granted, LeaseEvent, RevokeReason, Transition};
use super::lease::{Claim, ControlSlot, Lease, LeaseSlot, Owner};
use super::{LeaseError, Takeover};
use crate::registry::{LeaseGeneration, LeaseId, TerminalId};

#[derive(Debug, Default)]
pub(super) struct Slot {
    pub(super) current: Option<Lease>,
    epoch: Option<LeaseGeneration>,
}

impl Slot {
    fn grant(&mut self, slot: LeaseSlot, claim: Claim) -> Result<Lease, LeaseError> {
        let epoch = self
            .epoch
            .map_or_else(|| LeaseGeneration::new(1), LeaseGeneration::checked_next)
            .map_err(LeaseError::Exhausted)?;
        let lease = Lease {
            lease_id: claim.fresh,
            slot,
            epoch,
            owner: claim.owner,
        };
        self.epoch = Some(epoch);
        self.current = Some(lease.clone());
        Ok(lease)
    }

    fn take_owned(&mut self, owner: &Owner, lease: Option<LeaseId>) -> Option<Lease> {
        let held_by_owner = self.current.as_ref().is_some_and(|current| {
            current.owner == *owner && lease.is_none_or(|id| id == current.lease_id)
        });
        if held_by_owner {
            self.current.take()
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub(super) struct Table {
    terminal: TerminalId,
    pub(super) interact: Slot,
    pub(super) geometry: Slot,
    pub(super) observers: Vec<Lease>,
    pub(super) queue: VecDeque<Claim>,
}

const fn granted_event(slot: ControlSlot, lease: Lease, previous: Option<Lease>) -> LeaseEvent {
    match slot {
        ControlSlot::Interact => LeaseEvent::Acquired {
            lease,
            displaced: previous,
        },
        ControlSlot::Geometry => LeaseEvent::Geometry { lease, previous },
    }
}

const fn revoked(lease: Lease, reason: RevokeReason) -> LeaseEvent {
    LeaseEvent::Revoked {
        lease,
        reason,
        displaced_by: None,
    }
}

impl Table {
    pub(super) fn new(terminal: TerminalId) -> Self {
        Self {
            terminal,
            interact: Slot::default(),
            geometry: Slot::default(),
            observers: Vec::new(),
            queue: VecDeque::new(),
        }
    }

    const fn slot(&mut self, slot: ControlSlot) -> &mut Slot {
        match slot {
            ControlSlot::Interact => &mut self.interact,
            ControlSlot::Geometry => &mut self.geometry,
        }
    }

    const fn granted(&self, lease: Lease, events: Vec<LeaseEvent>) -> Granted {
        Granted {
            lease,
            transition: Transition::new(self.terminal, events),
        }
    }

    pub(super) fn observe(&mut self, claim: Claim) -> Result<Granted, LeaseError> {
        let lease = Lease {
            lease_id: claim.fresh,
            slot: LeaseSlot::Observe,
            epoch: LeaseGeneration::new(1).map_err(LeaseError::Exhausted)?,
            owner: claim.owner,
        };
        self.observers.push(lease.clone());
        let event = LeaseEvent::Acquired {
            lease: lease.clone(),
            displaced: None,
        };
        Ok(self.granted(lease, vec![event]))
    }

    pub(super) fn acquire(
        &mut self,
        slot: ControlSlot,
        claim: Claim,
    ) -> Result<Granted, LeaseError> {
        match self.slot(slot).current.clone() {
            Some(held) if held.owner == claim.owner => Ok(self.granted(held, Vec::new())),
            Some(holder) => {
                let queued = self
                    .queue
                    .iter()
                    .any(|waiting| waiting.owner == claim.owner);
                if slot == ControlSlot::Geometry && !queued {
                    self.queue.push_back(claim);
                }
                Err(LeaseError::Conflict {
                    holder: Box::new(holder),
                })
            }
            None => {
                self.queue.retain(|waiting| waiting.owner != claim.owner);
                let lease = self.slot(slot).grant(slot.into(), claim)?;
                Ok(self.granted(lease.clone(), vec![granted_event(slot, lease, None)]))
            }
        }
    }

    pub(super) fn take_over(
        &mut self,
        owner: Owner,
        takeover: &Takeover,
    ) -> Result<Granted, LeaseError> {
        let slot = takeover.slot();
        let current = self
            .slot(slot)
            .current
            .clone()
            .ok_or(LeaseError::NothingToTake)?;
        if current.owner == owner {
            return Err(LeaseError::SelfLease {
                held: Box::new(current),
            });
        }
        if current.epoch != takeover.expected() {
            let expected = takeover.expected();
            return Err(LeaseError::EpochMismatch {
                expected,
                current: Box::new(current),
            });
        }
        self.queue.retain(|waiting| waiting.owner != owner);
        let claim = Claim {
            owner,
            fresh: takeover.fresh(),
        };
        let lease = self.slot(slot).grant(slot.into(), claim)?;
        let events = vec![
            LeaseEvent::Revoked {
                lease: current.clone(),
                reason: RevokeReason::TakenOver,
                displaced_by: Some(lease.clone()),
            },
            granted_event(slot, lease.clone(), Some(current)),
        ];
        Ok(self.granted(lease, events))
    }

    pub(super) fn release(
        &mut self,
        owner: &Owner,
        lease: LeaseId,
    ) -> Result<Transition, LeaseError> {
        let mut events = Vec::new();
        if let Some(input) = self.interact.take_owned(owner, Some(lease)) {
            events.push(revoked(input, RevokeReason::Released));
        } else if let Some(grid) = self.geometry.take_owned(owner, Some(lease)) {
            self.queue.retain(|waiting| waiting.owner != *owner);
            events.push(revoked(grid.clone(), RevokeReason::Released));
            self.hand_on(grid, &mut events)?;
        } else if let Some(at) = self
            .observers
            .iter()
            .position(|watch| watch.lease_id == lease && watch.owner == *owner)
        {
            events.push(revoked(self.observers.remove(at), RevokeReason::Released));
        } else {
            return Err(LeaseError::NoLease);
        }
        Ok(Transition::new(self.terminal, events))
    }

    pub(super) fn disconnect(&mut self, owner: &Owner) -> Result<Transition, LeaseError> {
        let mut events = Vec::new();
        self.queue.retain(|waiting| waiting.owner != *owner);
        if let Some(input) = self.interact.take_owned(owner, None) {
            events.push(revoked(input, RevokeReason::Disconnected));
        }
        if let Some(grid) = self.geometry.take_owned(owner, None) {
            events.push(revoked(grid.clone(), RevokeReason::Disconnected));
            self.hand_on(grid, &mut events)?;
        }
        let (gone, kept) = std::mem::take(&mut self.observers)
            .into_iter()
            .partition(|watch| watch.owner == *owner);
        self.observers = kept;
        events.extend(
            gone.into_iter()
                .map(|watch: Lease| revoked(watch, RevokeReason::Disconnected)),
        );
        Ok(Transition::new(self.terminal, events))
    }

    /// Geometry passes to the earliest still-attached requester, never by focus or activity.
    fn hand_on(&mut self, previous: Lease, events: &mut Vec<LeaseEvent>) -> Result<(), LeaseError> {
        if let Some(next) = self.queue.pop_front() {
            let lease = self.geometry.grant(LeaseSlot::Geometry, next)?;
            events.push(LeaseEvent::Geometry {
                lease,
                previous: Some(previous),
            });
        }
        Ok(())
    }
}

//! Write and resize admission against the current leases, and the write-boundary fence.

use serde::Serialize;

use super::LeaseError;
use super::lease::{Lease, LeaseSlot, Owner};
use super::table::Table;
use crate::registry::LeaseId;

/// Every lease on a terminal and the published geometry hand-on order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct LeaseView {
    /// Interact lease, when held.
    pub interact: Option<Lease>,
    /// Geometry lease, when held.
    pub geometry: Option<Lease>,
    /// Observe leases in attach order.
    pub observers: Vec<Lease>,
    /// Interact-capable owners waiting for geometry, earliest request first.
    pub geometry_queue: Vec<Owner>,
}

impl Table {
    pub(super) const fn is_unleased(&self) -> bool {
        self.interact.current.is_none()
            && self.geometry.current.is_none()
            && self.observers.is_empty()
    }

    pub(super) fn check_write(
        &self,
        owner: &Owner,
        lease: Option<LeaseId>,
    ) -> Result<Option<Lease>, LeaseError> {
        match (&self.interact.current, lease) {
            (_, None) if self.is_unleased() => Ok(None),
            (Some(current), Some(id)) if current.lease_id == id && current.owner == *owner => {
                Ok(Some(current.clone()))
            }
            _ => Err(LeaseError::NoLease),
        }
    }

    pub(super) fn check_resize(&self, owner: &Owner, lease: LeaseId) -> Result<Lease, LeaseError> {
        if self
            .observers
            .iter()
            .any(|watch| watch.lease_id == lease && watch.owner == *owner)
        {
            return Err(LeaseError::ObserveOnly);
        }
        match &self.geometry.current {
            None => Err(LeaseError::NoLease),
            Some(current) if current.lease_id == lease && current.owner == *owner => {
                Ok(current.clone())
            }
            Some(current) => Err(LeaseError::NotGeometryOwner {
                owner: Box::new(current.clone()),
            }),
        }
    }

    /// Fails unless the lease an operation was admitted under still holds its slot.
    pub(super) fn fence(&self, admitted: Option<&Lease>) -> Result<(), LeaseError> {
        let Some(admitted) = admitted else {
            return if self.is_unleased() {
                Ok(())
            } else {
                Err(LeaseError::NoLease)
            };
        };
        let current = match admitted.slot {
            LeaseSlot::Interact => self.interact.current.as_ref(),
            LeaseSlot::Geometry => self.geometry.current.as_ref(),
            LeaseSlot::Observe => self.observers.iter().find(|watch| *watch == admitted),
        };
        if current == Some(admitted) {
            Ok(())
        } else {
            Err(LeaseError::Revoked {
                lease: admitted.lease_id,
            })
        }
    }

    pub(super) fn view(&self) -> LeaseView {
        LeaseView {
            interact: self.interact.current.clone(),
            geometry: self.geometry.current.clone(),
            observers: self.observers.clone(),
            geometry_queue: self
                .queue
                .iter()
                .map(|waiting| waiting.owner.clone())
                .collect(),
        }
    }
}

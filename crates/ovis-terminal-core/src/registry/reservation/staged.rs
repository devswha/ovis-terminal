//! Identity reservations staged into the caller's atomic lifecycle transaction.

use std::num::NonZeroU64;

use ovis_terminal_store::{CasWrite, Payload, ResourceKey, ResourceLookup, Snapshot, Version};

use super::{COUNTER_KIND, CounterText, IdentityReservation, RESERVATION_KIND, ReservationError};
use crate::registry::DurableId;

/// A provisional identity and the mandatory writes that make it durable.
#[derive(Debug)]
pub struct ReservedIdentity<T> {
    id: T,
    writes: Vec<CasWrite>,
}

impl<T> ReservedIdentity<T> {
    /// Transfers the provisional identity and its writes to one commit unit.
    ///
    /// The identity must not be published until every returned write commits.
    pub fn into_parts(self) -> (T, Vec<CasWrite>) {
        (self.id, self.writes)
    }
}

impl IdentityReservation<'_> {
    /// Stages one family-local reservation without exposing or opening a writer.
    ///
    /// Use inside `Coordinator::commit_unit`; the reservation writes share its
    /// transaction with the new resource, operation outcome, and lifecycle event.
    /// Reserve at most one identity of a given family per supplied snapshot.
    ///
    /// # Errors
    ///
    /// Returns typed allocator exhaustion, corrupt counter, or store read failures.
    pub fn stage<T: DurableId>(
        snapshot: &Snapshot,
    ) -> Result<ReservedIdentity<T>, ReservationError> {
        let key = ResourceKey::new(COUNTER_KIND, T::FAMILY);
        let (current, version) = match snapshot.resource(&key)? {
            ResourceLookup::Absent => (0, Version::ABSENT),
            ResourceLookup::Live { version, value, .. } => {
                let counter: CounterText = serde_json::from_slice(value.as_bytes())
                    .map_err(ReservationError::InvalidCounter)?;
                if counter.0 == 0 {
                    return Err(ReservationError::AllocatorUnavailable);
                }
                (counter.0, version)
            }
            _ => return Err(ReservationError::AllocatorUnavailable),
        };
        let next = current
            .checked_add(1)
            .ok_or(ReservationError::CounterOverflow)?;
        let id = T::from_sequence(NonZeroU64::new(next).ok_or(ReservationError::CounterOverflow)?);
        let counter =
            serde_json::to_vec(&CounterText(next)).map_err(ReservationError::Serialization)?;
        Ok(ReservedIdentity {
            id,
            writes: vec![
                CasWrite::new(key, version, Some(Payload::from(counter))),
                CasWrite::new(
                    ResourceKey::new(RESERVATION_KIND, id.to_string()),
                    Version::ABSENT,
                    Some(Payload::from("reserved")),
                ),
            ],
        })
    }
}

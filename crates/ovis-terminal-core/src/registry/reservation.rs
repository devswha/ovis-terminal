//! Durable identity reservation before a resource can be published.

use std::str::FromStr as _;

use ovis_terminal_store::{
    CasWrite, EventKind, Mutation, NewEvent, OpId, Payload, ResourceKey, ResourceLookup, Snapshot,
    Store, StoreError, Version,
};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::registry::{DurableId, IdentityError, MachineId};

mod staged;
pub use staged::ReservedIdentity;

const COUNTER_KIND: &str = "identity_allocator";
const RESERVATION_KIND: &str = "identity_reservation";
const MACHINE_IDENTITY_KIND: &str = "machine_identity";

#[derive(Debug, Serialize, Deserialize)]
struct CounterText(#[serde(with = "decimal_string")] u64);

mod decimal_string {
    use super::*;

    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's with serializer requires a reference to the field"
    )]
    pub(super) fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        let value =
            u64::from_str(&raw).map_err(|_| D::Error::custom(IdentityError::InvalidIdentity))?;
        if value.to_string() != raw {
            return Err(D::Error::custom(IdentityError::InvalidIdentity));
        }
        Ok(value)
    }
}

/// Failure while durably reserving an opaque ID.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReservationError {
    /// A typed identity failed validation before it could be reserved.
    #[error(transparent)]
    Identity(#[from] IdentityError),
    /// The allocator counter has reached its maximum value.
    #[error("the identity allocator counter is exhausted")]
    CounterOverflow,
    /// A persisted counter is not a canonical decimal string.
    #[error("the persisted identity counter is malformed")]
    InvalidCounter(#[source] serde_json::Error),
    /// The allocator resource was deleted or has an unexpected shape.
    #[error("the persisted identity allocator is not readable")]
    AllocatorUnavailable,
    /// A serialized identity reservation could not be encoded or decoded.
    #[error("the identity reservation could not be encoded or decoded")]
    Serialization(#[source] serde_json::Error),
    /// The durable store rejected the reservation transaction.
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl ReservationError {
    /// Stable machine-readable error kind.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Identity(error) => error.kind(),
            Self::CounterOverflow => "counter_overflow",
            Self::InvalidCounter(_) | Self::AllocatorUnavailable => "identity_store_invalid",
            Self::Serialization(_) => "persistence_failed",
            Self::Store(error) => error.kind(),
        }
    }

    /// Recovery guidance for the error kind.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Identity(error) => error.hint(),
            Self::CounterOverflow => "allocate identities in a new store lifetime",
            Self::InvalidCounter(_) | Self::AllocatorUnavailable => {
                "inspect the durable allocator record and recovery evidence"
            }
            Self::Serialization(_) => "inspect the failed identity serialization boundary",
            Self::Store(error) => error.hint(),
        }
    }
}

/// Reserves monotonically allocated opaque IDs in the durable store.
#[derive(Debug)]
pub struct IdentityReservation<'a> {
    store: &'a Store,
}

impl<'a> IdentityReservation<'a> {
    /// Binds the allocator to the core's durable store.
    pub const fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Loads the installation identity or commits a `UUIDv7` reservation before returning it.
    ///
    /// # Errors
    ///
    /// Returns `identity_store_invalid` or a typed store failure when identity
    /// persistence fails or another core wins the initial compare-and-swap.
    pub async fn load_or_reserve_machine(&self, op: OpId) -> Result<MachineId, ReservationError> {
        let machine_key = ResourceKey::new(MACHINE_IDENTITY_KIND, "local");
        let snapshot = self.store.snapshot()?;
        match snapshot.resource(&machine_key)? {
            ResourceLookup::Live {
                value,
                revision: _,
                version: _,
            } => serde_json::from_slice(value.as_bytes()).map_err(ReservationError::Serialization),
            ResourceLookup::Absent => {
                let id = MachineId::new(Uuid::now_v7())?;
                let id_bytes = serde_json::to_vec(&id).map_err(ReservationError::Serialization)?;
                let event = NewEvent::new(
                    EventKind::new("identity.reserved"),
                    Payload::from(id_bytes.clone()),
                );
                let mutation = Mutation::new(op, Payload::from(id_bytes.clone()), event)
                    .compare_and_swap(CasWrite::new(
                        machine_key,
                        Version::ABSENT,
                        Some(Payload::from(id_bytes.clone())),
                    ))
                    .compare_and_swap(CasWrite::new(
                        ResourceKey::new(RESERVATION_KIND, id.to_string()),
                        Version::ABSENT,
                        Some(Payload::from("reserved")),
                    ));
                self.store.submit(mutation)?.outcome().await?;
                Ok(id)
            }
            _ => Err(ReservationError::AllocatorUnavailable),
        }
    }

    /// Commits a family-local ID reservation before returning it to the caller.
    ///
    /// # Errors
    ///
    /// Returns `counter_overflow`, `identity_store_invalid` or `persistence_failed` if
    /// the durable compare-and-swap transaction cannot reserve the next identity.
    pub async fn reserve<T: DurableId>(&self, op: OpId) -> Result<T, ReservationError> {
        let snapshot = self.store.snapshot()?;
        let (id, writes) = Self::stage::<T>(&snapshot)?.into_parts();
        let id_bytes = serde_json::to_vec(&id).map_err(ReservationError::Serialization)?;
        let event = NewEvent::new(
            EventKind::new("identity.reserved"),
            Payload::from(id_bytes.clone()),
        );
        let mut mutation = Mutation::new(op, Payload::from(id_bytes), event);
        for write in writes {
            mutation = mutation.compare_and_swap(write);
        }
        self.store.submit(mutation)?.outcome().await?;
        Ok(id)
    }

    /// Reports whether the exact typed identity has a committed reservation.
    ///
    /// # Errors
    ///
    /// Returns `persistence_failed` when the snapshot cannot read the reservation.
    pub fn is_reserved<T: DurableId>(
        &self,
        snapshot: &Snapshot,
        id: T,
    ) -> Result<bool, ReservationError> {
        match snapshot.resource(&ResourceKey::new(RESERVATION_KIND, id.to_string()))? {
            ResourceLookup::Live { .. } => Ok(true),
            ResourceLookup::Absent | ResourceLookup::Tombstone { .. } => Ok(false),
            _ => Err(ReservationError::AllocatorUnavailable),
        }
    }
}

#[cfg(test)]
mod tests;

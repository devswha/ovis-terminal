//! Durable floor records and the floors of every class at one snapshot.

use ovis_terminal_protocol::domain::documents_retention::{
    RetentionClass, RetentionFloor, RetentionFloors, RetentionPosition,
};
use ovis_terminal_protocol::schema::scalar::Revision as WireRevision;
use ovis_terminal_store::{
    EventPage, OpId, ReceiptLookup, ResourceKey, ResourceLookup, Revision, Snapshot, Version,
};
use serde::{Deserialize, Serialize};

use super::RetentionError;

/// Receipt id that no commit uses: every coordinator commit is keyed by a `UUIDv7`, so this
/// lookup can only report whether receipts were evicted and below which revision.
const RECEIPT_FLOOR_PROBE: &str = "retention-floor-probe";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FloorRecord {
    pub floor: RetentionPosition,
}

pub(super) fn key(class: RetentionClass) -> ResourceKey {
    ResourceKey::new("retention_floor", class.token())
}

/// The class's committed floor record and its compare-and-swap version.
pub(super) fn recorded(
    snapshot: &Snapshot,
    class: RetentionClass,
) -> Result<(RetentionPosition, Version), RetentionError> {
    match snapshot.resource(&key(class))? {
        ResourceLookup::Live { value, version, .. } => {
            let record: FloorRecord = serde_json::from_slice(value.as_bytes())?;
            Ok((record.floor, version))
        }
        ResourceLookup::Absent => Ok((RetentionPosition::new(0), Version::ABSENT)),
        _ => Err(RetentionError::UnsupportedRecord),
    }
}

/// The floor the store itself advanced while trimming events or receipts.
fn trimmed(snapshot: &Snapshot, class: RetentionClass) -> Result<u64, RetentionError> {
    let floor = match class {
        RetentionClass::Events => match snapshot.events_after(Revision::ZERO, 0)? {
            EventPage::Expired { floor } => floor,
            EventPage::Events(_) => Revision::ZERO,
            _ => return Err(RetentionError::UnsupportedRecord),
        },
        RetentionClass::Receipts => match snapshot.receipt(&OpId::new(RECEIPT_FLOOR_PROBE))? {
            ReceiptLookup::NotRetained { floor } => floor,
            ReceiptLookup::Absent => Revision::ZERO,
            _ => return Err(RetentionError::UnsupportedRecord),
        },
        _ => Revision::ZERO,
    };
    Ok(floor.get())
}

/// Floors of every class at the snapshot's revision; for events and receipts the higher of
/// the swept floor and the store's own trim floor.
///
/// # Errors
///
/// Returns `invalid_record` for an unreadable floor record or a typed store failure.
pub fn floors(snapshot: &Snapshot) -> Result<RetentionFloors, RetentionError> {
    let mut floors = Vec::with_capacity(RetentionClass::ALL.len());
    for class in RetentionClass::ALL {
        let (swept, _) = recorded(snapshot, class)?;
        let floor = swept.get().max(trimmed(snapshot, class)?);
        floors.push(RetentionFloor::new(class, RetentionPosition::new(floor)));
    }
    Ok(RetentionFloors::new(
        WireRevision::new(snapshot.revision().get()),
        floors,
    ))
}

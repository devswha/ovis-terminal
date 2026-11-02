//! The wire data of a `lease.*` event, as `events.subscribe` delivers it: the published
//! `LeaseEventData` shape, checked against it before it leaves the runtime.

use ovis_terminal_core::leases::LeaseEvent;
use ovis_terminal_protocol::schema::events::LeaseEventData;
use ovis_terminal_server::streams::lease_record;
use serde_json::{Value, json};

/// `{lease, displaced_by, reason}` of `event`, validated as `LeaseEventData`.
pub(super) fn payload(event: &LeaseEvent) -> Result<Value, serde_json::Error> {
    let data = match event {
        LeaseEvent::Revoked {
            lease,
            reason,
            displaced_by,
        } => json!({
            "lease": lease_record(lease),
            "displaced_by": displaced_by.as_ref().map(lease_record),
            "reason": reason,
        }),
        LeaseEvent::Acquired { lease, .. } | LeaseEvent::Geometry { lease, .. } => {
            json!({"lease": lease_record(lease), "displaced_by": null, "reason": null})
        }
        _ => json!({}),
    };
    let checked: LeaseEventData = serde_json::from_value(data)?;
    serde_json::to_value(checked)
}

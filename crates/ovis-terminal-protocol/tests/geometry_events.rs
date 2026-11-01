//! The geometry transition events and `stale_geometry` are published, and their data keeps
//! every counter a canonical decimal string.

use ovis_terminal_protocol::schema::events::GeometryEventData;
use serde_json::json;

#[test]
fn geometry_events_round_trip_the_largest_epoch_and_refuse_a_numeric_one() {
    let record = json!({
        "terminal_id": "terminal:7",
        "epoch": u64::MAX.to_string(),
        "geometry": {"cols": 120, "rows": 40},
        "previous": {"cols": 80, "rows": 24},
        "owner_lease": "lease:2",
        "output_offset": "18446744073709551615",
        "revision": null,
    });
    let decoded: Result<GeometryEventData, _> = serde_json::from_value(record.clone());
    let encoded = decoded
        .ok()
        .and_then(|data| serde_json::to_value(data).ok());
    assert_eq!(encoded, Some(record.clone()));
    let mut numeric = record;
    if let Some(fields) = numeric.as_object_mut() {
        fields.insert("epoch".to_owned(), json!(7));
    }
    assert!(serde_json::from_value::<GeometryEventData>(numeric).is_err());
}

#[test]
fn the_catalog_publishes_the_geometry_events_and_stale_geometry() {
    let document = ovis_terminal_protocol::schema::render_openrpc().unwrap_or_default();
    for name in [
        "geometry.changed",
        "geometry.checkpoint",
        "geometry.input_ready",
        "stale_geometry",
    ] {
        assert!(
            document.contains(&format!("\"{name}\"")),
            "{name} is not published"
        );
    }
}

use serde_json::{Value, json};

use super::TestResult;
use crate::domain::graphics::{
    GraphicsCaps, GraphicsDroppedData, GraphicsEvictedData, GraphicsProtocol, SequenceAdmission,
};
use crate::schema::limits::Limits;
use crate::schema::openrpc_document;
use crate::schema::scalar::{ByteCount, ByteOffset, Count};

const DROPPED_FIELDS: [&str; 5] = [
    "limit_bytes",
    "output_offset",
    "protocol",
    "reason",
    "sequence_bytes",
];
const EVICTED_FIELDS: [&str; 5] = [
    "evicted_bytes",
    "evicted_sequences",
    "limit_bytes",
    "reason",
    "retained_bytes",
];

fn with_payload(mut notice: Value) -> Result<Value, Box<dyn std::error::Error>> {
    notice
        .as_object_mut()
        .ok_or("a notice is a JSON object")?
        .insert("payload".to_owned(), json!("iVBORw0KGgoAAAANSUhEUgAAAAE="));
    Ok(notice)
}

#[test]
fn the_event_catalog_publishes_both_graphics_notices() -> TestResult {
    let document = openrpc_document()?;
    let events = document
        .get("x-events")
        .and_then(Value::as_array)
        .ok_or("x-events")?;
    let schema_of = |name: &str| {
        events
            .iter()
            .find(|event| event.get("name") == Some(&json!(name)))
            .and_then(|event| event.pointer("/schema/$ref"))
            .cloned()
    };
    assert_eq!(
        schema_of("graphics.dropped"),
        Some(json!("#/components/schemas/GraphicsDroppedData"))
    );
    assert_eq!(
        schema_of("graphics.evicted"),
        Some(json!("#/components/schemas/GraphicsEvictedData"))
    );
    let schemas = document.pointer("/components/schemas").ok_or("schemas")?;
    let tokens = |name: &str| -> Vec<Value> {
        schemas
            .pointer(&format!("/{name}/oneOf"))
            .and_then(Value::as_array)
            .map(|variants| {
                variants
                    .iter()
                    .filter_map(|v| v.get("const").cloned())
                    .collect()
            })
            .unwrap_or_default()
    };
    assert_eq!(tokens("GraphicsDropReason"), vec![json!("too_large")]);
    assert_eq!(tokens("GraphicsEvictReason"), vec![json!("store_full")]);
    Ok(())
}

#[test]
fn notice_schemas_are_closed_objects_of_their_typed_fields_only() -> TestResult {
    let document = openrpc_document()?;
    for (name, fields) in [
        ("GraphicsDroppedData", DROPPED_FIELDS),
        ("GraphicsEvictedData", EVICTED_FIELDS),
    ] {
        let schema = document
            .pointer(&format!("/components/schemas/{name}"))
            .ok_or(name)?;
        let mut properties: Vec<&str> = schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|object| object.keys().map(String::as_str).collect())
            .unwrap_or_default();
        let mut required: Vec<&str> = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        properties.sort_unstable();
        required.sort_unstable();
        assert_eq!(
            schema.get("additionalProperties"),
            Some(&json!(false)),
            "{name}"
        );
        assert_eq!(properties, fields, "{name}");
        assert_eq!(required, fields, "{name}");
    }
    Ok(())
}

#[test]
fn a_notice_carrying_any_extra_field_is_refused() -> TestResult {
    let caps = GraphicsCaps::advertised(&Limits::v1());
    let widest = (ByteOffset::new(u64::MAX), ByteCount::new(u64::MAX));
    let SequenceAdmission::Dropped(dropped) = caps.admit(GraphicsProtocol::Kitty, widest) else {
        return Err("u64::MAX bytes exceed the cap".into());
    };
    let evicted = GraphicsEvictedData::store_full(
        caps,
        (Count::new(1), ByteCount::new(9)),
        ByteCount::new(0),
    );
    let dropped_json = serde_json::to_value(&dropped)?;
    let evicted_json = serde_json::to_value(&evicted)?;
    assert_eq!(
        serde_json::from_value::<GraphicsDroppedData>(dropped_json.clone())?,
        dropped
    );
    assert_eq!(
        serde_json::from_value::<GraphicsEvictedData>(evicted_json.clone())?,
        evicted
    );
    assert!(serde_json::from_value::<GraphicsDroppedData>(with_payload(dropped_json)?).is_err());
    assert!(serde_json::from_value::<GraphicsEvictedData>(with_payload(evicted_json)?).is_err());
    Ok(())
}

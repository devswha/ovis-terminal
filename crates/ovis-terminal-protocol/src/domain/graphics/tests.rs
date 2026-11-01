use serde_json::{Value, json};

use super::{
    GraphicsCaps, GraphicsDroppedData, GraphicsEvictedData, GraphicsProtocol, SequenceAdmission,
};
use crate::schema::limits::Limits;
use crate::schema::openrpc_document;
use crate::schema::scalar::{ByteCount, ByteOffset, Count};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const CAPS: [&str; 2] = [
    "graphics_sequence_max_bytes",
    "graphics_terminal_store_max_bytes",
];

#[test]
fn hello_limits_carry_both_graphics_caps_as_decimal_strings() -> TestResult {
    let encoded = serde_json::to_value(Limits::v1())?;
    let caps: Vec<Option<&Value>> = CAPS.iter().map(|name| encoded.get(*name)).collect();
    assert_eq!(
        caps,
        vec![Some(&json!("8388608")), Some(&json!("33554432"))]
    );
    assert_eq!(serde_json::from_value::<Limits>(encoded)?, Limits::v1());
    Ok(())
}

fn with_cap(name: &str, value: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let mut limits = serde_json::to_value(Limits::v1())?;
    *limits.get_mut(name).ok_or_else(|| name.to_owned())? = value;
    Ok(limits)
}

#[test]
fn graphics_caps_round_trip_u64_max_and_refuse_numbers() -> TestResult {
    for name in CAPS {
        let widest = with_cap(name, json!("18446744073709551615"))?;
        let decoded = serde_json::from_value::<Limits>(widest)?;
        assert_eq!(
            serde_json::to_value(decoded)?.get(name),
            Some(&json!(u64::MAX.to_string()))
        );
        let numeric = with_cap(name, json!(8_388_608))?;
        assert!(serde_json::from_value::<Limits>(numeric).is_err(), "{name}");
    }
    Ok(())
}

#[test]
fn the_cap_is_admitted_and_one_byte_more_is_dropped_whole() -> TestResult {
    let caps = GraphicsCaps::advertised(&Limits::v1());
    let at = |bytes| (ByteOffset::new(4096), ByteCount::new(bytes));
    assert_eq!(
        caps.admit(GraphicsProtocol::Kitty, at(8 * 1024 * 1024)),
        SequenceAdmission::Admitted
    );
    let SequenceAdmission::Dropped(notice) = caps.admit(GraphicsProtocol::Sixel, at(8_388_609))
    else {
        return Err("one byte over the cap must be dropped".into());
    };
    assert_eq!(
        serde_json::to_value(&notice)?,
        json!({
            "protocol": "sixel",
            "reason": "too_large",
            "output_offset": "4096",
            "sequence_bytes": "8388609",
            "limit_bytes": "8388608",
        })
    );
    assert_eq!(
        serde_json::from_value::<GraphicsDroppedData>(serde_json::to_value(&notice)?)?,
        notice
    );
    Ok(())
}

#[test]
fn an_eviction_notice_names_store_full_and_the_store_cap() -> TestResult {
    let caps = GraphicsCaps::advertised(&Limits::v1());
    let notice = GraphicsEvictedData::store_full(
        caps,
        (Count::new(2), ByteCount::new(9_000_000)),
        ByteCount::new(u64::MAX),
    );
    assert_eq!(
        serde_json::to_value(&notice)?,
        json!({
            "reason": "store_full",
            "evicted_sequences": "2",
            "evicted_bytes": "9000000",
            "retained_bytes": "18446744073709551615",
            "limit_bytes": "33554432",
        })
    );
    Ok(())
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
fn every_layer_that_carries_a_sequence_can_hold_the_cap() {
    let limits = Limits::v1();
    let sequence = limits.graphics_sequence_max_bytes.get();
    let store = limits.graphics_terminal_store_max_bytes.get();
    let base64 = sequence.div_ceil(3) * 4;
    assert!(
        base64 < limits.max_frame_bytes.get(),
        "plain-mode base64 of one sequence fits a frame"
    );
    assert!(
        sequence <= store,
        "one admitted sequence fits the terminal store"
    );
    assert!(
        store <= limits.output_ring_bytes.get(),
        "the store fits the raw replay ring"
    );
    assert!(
        store <= limits.session_retained_bytes.get(),
        "the store fits the session budget"
    );
}

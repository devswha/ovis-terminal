//! Geometry epochs and events on the wire: decimal strings only.

use ovis_terminal_core::geometry::{GeometryEpoch, GeometryEvent};
use proptest::prelude::*;
use serde_json::{Value, json};

use super::TestResult;
use super::rig::{Rig, dispatched, holder_target, op, screen};

#[test]
fn the_largest_epoch_round_trips_as_a_decimal_string_and_numbers_are_refused() -> TestResult {
    // Given the largest epoch.
    let epoch = GeometryEpoch::new(u64::MAX);

    // When it is encoded and decoded, and a numeric form is decoded.
    let encoded = serde_json::to_value(epoch)?;
    let decoded: GeometryEpoch = serde_json::from_value(encoded.clone())?;
    let numeric = serde_json::from_value::<GeometryEpoch>(json!(7));

    // Then only the canonical string form exists.
    assert_eq!(encoded, json!("18446744073709551615"));
    assert_eq!(decoded, epoch);
    assert!(numeric.is_err());
    Ok(())
}

#[test]
fn committed_events_carry_counters_as_decimal_strings_and_round_trip() -> TestResult {
    // Given a completed transition.
    let mut rig = Rig::new(screen(holder_target(1), 80, 24)?)?;
    let dispatch = dispatched(rig.resize(&op(), (100, 30))??)?;
    let change = dispatch.event().clone();
    rig.start(dispatch)?;
    let settled = rig.acknowledged(GeometryEpoch::new(1))?;

    // When every event is encoded and decoded.
    let events: Vec<GeometryEvent> = std::iter::once(change).chain(settled.events).collect();
    let encoded = serde_json::to_value(&events)?;
    let decoded: Vec<GeometryEvent> = serde_json::from_value(encoded.clone())?;

    // Then epochs, offsets, revisions and sequences are strings and nothing is lost.
    assert_eq!(decoded, events);
    let counters = ["epoch", "from_offset", "offset", "revision", "changed_seq"];
    let mut seen = 0_u32;
    for body in encoded
        .as_array()
        .ok_or("not an array")?
        .iter()
        .filter_map(Value::as_object)
    {
        for fields in body.values().filter_map(Value::as_object) {
            for name in counters {
                if let Some(value) = fields.get(name) {
                    assert!(
                        value.is_string(),
                        "{name} must be a decimal string: {value}"
                    );
                    seen = seen.saturating_add(1);
                }
            }
        }
    }
    assert_eq!(seen, 8);
    Ok(())
}

proptest! {
    #[test]
    fn every_epoch_round_trips(value in any::<u64>()) {
        let encoded = serde_json::to_value(GeometryEpoch::new(value))
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(&encoded, &json!(value.to_string()));
        let decoded: GeometryEpoch = serde_json::from_value(encoded)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(decoded.get(), value);
    }
}

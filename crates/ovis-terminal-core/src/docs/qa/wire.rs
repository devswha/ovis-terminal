//! Wire forms: every counter is a canonical decimal string, never a JSON number.

use ovis_terminal_protocol::domain::documents_retention::{
    Retained, RetentionClass, RetentionFloor, RetentionPolicy, RetentionPosition,
};
use ovis_terminal_protocol::schema::ids::DocName;
use ovis_terminal_protocol::schema::methods::doc::{DocEntry, DocSetParams, DocSetResult};
use ovis_terminal_protocol::schema::scalar::{EventSeq, RecordVersion};
use proptest::prelude::*;
use serde_json::json;

use super::fixture::{TestResult, at};
use crate::docs::DocChanged;

#[test]
fn u64_max_versions_sequences_and_floors_round_trip_as_exact_decimal_strings() -> TestResult {
    // Given: the largest representable version, sequence and position.
    let result = DocSetResult::new(RecordVersion::new(u64::MAX), EventSeq::new(u64::MAX));
    let floor = RetentionFloor::new(RetentionClass::Events, RetentionPosition::new(u64::MAX));

    // When: they are encoded and decoded.
    let encoded = (serde_json::to_value(&result)?, serde_json::to_value(floor)?);

    // Then: the text is exact and decoding returns the same values.
    assert_eq!(
        encoded.0,
        json!({"version": "18446744073709551615", "seq": "18446744073709551615"})
    );
    assert_eq!(
        encoded.1,
        json!({"class": "events", "floor": "18446744073709551615"})
    );
    assert_eq!(serde_json::from_value::<DocSetResult>(encoded.0)?, result);
    assert_eq!(serde_json::from_value::<RetentionFloor>(encoded.1)?, floor);
    Ok(())
}

#[test]
fn numeric_counters_are_rejected() {
    // Given/When: a doc.set whose version, and a floor whose position, are JSON numbers.
    let params = json!({"op_id": "0191f1a2-0000-7000-8000-000000000050", "name": "x", "version": 0, "value": 1});
    let floor = json!({"class": "events", "floor": 3});

    // Then: neither decodes.
    assert!(serde_json::from_value::<DocSetParams>(params).is_err());
    assert!(serde_json::from_value::<RetentionFloor>(floor).is_err());
}

#[test]
fn the_change_event_carries_name_and_version_but_never_the_value() -> TestResult {
    let change = DocChanged::new(DocEntry::new(
        DocName::new("shared".to_owned()),
        RecordVersion::new(7),
    ));
    assert_eq!(
        serde_json::to_value(change)?,
        json!({"name": "shared", "version": "7"})
    );
    Ok(())
}

#[test]
fn the_policy_advertises_every_class_with_its_v1_bound() -> TestResult {
    let policy = serde_json::to_value(RetentionPolicy::v1())?;
    let classes: Vec<&str> = at(&policy, "/limits")
        .as_array()
        .ok_or("limits")?
        .iter()
        .filter_map(|limit| at(limit, "/class").as_str())
        .collect();
    assert_eq!(
        classes,
        [
            "output_history",
            "events",
            "receipts",
            "plugin_runs",
            "finished_terminals"
        ]
    );
    assert_eq!(
        at(&policy, "/limits/1"),
        &json!({"class": "events", "unit": "records", "capacity": "10000", "max_age_ms": null})
    );
    assert_eq!(at(&policy, "/limits/2/max_age_ms"), "86400000");
    Ok(())
}

proptest! {
    #[test]
    fn a_floor_reports_expired_and_gap_exactly_below_itself(floor in any::<u64>(), at in any::<u64>()) {
        let floor = RetentionFloor::new(RetentionClass::Receipts, RetentionPosition::new(floor));
        let position = RetentionPosition::new(at);
        let expected_lookup = if at < floor.floor.get() {
            Retained::Expired { floor: floor.floor }
        } else {
            Retained::Available
        };
        prop_assert_eq!(floor.lookup(position), expected_lookup);
        let gap = at.checked_add(1).is_some_and(|next| next < floor.floor.get());
        prop_assert_eq!(floor.resume_after(position) == Retained::Available, !gap);
        let encoded = serde_json::to_value(floor.resume_after(position)).map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(serde_json::from_value::<Retained>(encoded).ok(), Some(floor.resume_after(position)));
    }
}

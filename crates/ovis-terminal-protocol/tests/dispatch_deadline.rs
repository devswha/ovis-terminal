//! Deadline failures preserve their typed recovery handle and exact wire counters.

use ovis_terminal_protocol::errors::{BoundaryError, ErrorData, OpId};
use ovis_terminal_protocol::schema::scalar::DurationMs;
use ovis_terminal_protocol::schema::{breaking_changes, check_compatibility, openrpc_document};
use proptest::prelude::*;
use serde_json::{Value, json};

#[cfg(test)]
fn at<'a>(value: &'a Value, path: &str) -> &'a Value {
    value.pointer(path).unwrap_or(&Value::Null)
}

#[test]
fn deadline_catalog_and_payload_are_additive() {
    // Given the published document and a prior peer without the deadline extension.
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../schema/fixtures/deadline-additive.json"
    ))
    .expect("fixture");
    let current = openrpc_document().expect("document");
    let kind = at(&fixture, "/kind").as_str().expect("kind");
    let mut prior = current.clone();
    prior
        .pointer_mut("/x-error-kinds")
        .expect("catalog")
        .as_array_mut()
        .expect("catalog")
        .retain(|entry| at(entry, "/kind") != kind);
    prior
        .pointer_mut("/components/schemas/ErrorKindName/enum")
        .expect("kind enum")
        .as_array_mut()
        .expect("kinds")
        .retain(|entry| entry != kind);
    for field in at(&fixture, "/added_fields").as_array().expect("fields") {
        prior
            .pointer_mut("/components/schemas/ErrorData/properties")
            .expect("properties")
            .as_object_mut()
            .expect("properties")
            .remove(field.as_str().expect("field"));
    }
    // When checking additive publication and accidental removal.
    check_compatibility(&prior, &current).expect("additive");
    let breaks = breaking_changes(&current, &prior).expect("comparison");
    // Then addition is permitted, but retiring the kind is rejected.
    assert!(
        at(&current, "/x-error-kinds")
            .as_array()
            .expect("catalog")
            .iter()
            .any(|entry| at(entry, "/kind") == kind)
    );
    assert!(breaks.iter().any(|change| change.path
        == at(&fixture, "/expect/removal_path").as_str().expect("path")
        && change.rule.token() == at(&fixture, "/expect/removal_rule").as_str().expect("rule")));
}

#[test]
fn deadline_roundtrips_maximum_without_numeric_coercion() {
    // Given a deadline failure with the largest wire duration.
    let error = BoundaryError::deadline_exceeded(
        OpId::new("01800000-0000-7000-8000-000000000001".into()),
        DurationMs::new(u64::MAX),
        DurationMs::new(u64::MAX),
    );
    // When a client decodes the error data.
    let encoded = serde_json::to_value(error.data()).expect("encode");
    let decoded: ErrorData = serde_json::from_value(encoded.clone()).expect("decode");
    // Then neither duration loses precision, and numeric durations are rejected.
    assert_eq!(at(&encoded, "/deadline_ms"), "18446744073709551615");
    assert_eq!(at(&encoded, "/elapsed_ms"), "18446744073709551615");
    assert_eq!(&decoded, error.data());
    let mut numeric = encoded;
    *numeric.get_mut("deadline_ms").expect("deadline") = json!(u64::MAX);
    assert!(serde_json::from_value::<ErrorData>(numeric).is_err());
}

proptest! {
    #[test]
    fn deadline_payload_roundtrips(deadline in any::<u64>(), elapsed in any::<u64>()) {
        let error = BoundaryError::deadline_exceeded(
            OpId::new("01800000-0000-7000-8000-000000000001".into()),
            DurationMs::new(deadline),
            DurationMs::new(elapsed),
        );
        let bytes = serde_json::to_vec(error.data()).expect("encode");
        let decoded: ErrorData = serde_json::from_slice(&bytes).expect("decode");
        prop_assert_eq!(&decoded, error.data());
    }
}

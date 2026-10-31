//! Advertised count limits travel as canonical decimal strings like every other counter.

use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_protocol::schema::scalar::Count;
use proptest::prelude::*;
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const COUNTS: [&str; 4] = [
    "event_replay_events",
    "history_lines",
    "max_in_flight_requests",
    "queued_operations",
];

proptest! {
    #[test]
    fn counts_round_trip_every_value_as_a_decimal_string(value in any::<u64>()) {
        let encoded = serde_json::to_value(Count::new(value)).map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(&encoded, &json!(value.to_string()));
        let decoded: Count = serde_json::from_value(encoded).map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(decoded.get(), value);
    }
}

#[test]
fn advertised_count_limits_are_decimal_strings_and_numbers_are_refused() -> TestResult {
    // Given the advertised v1 limits as hello serializes them.
    let encoded = serde_json::to_value(Limits::v1())?;
    // When each count is read back, and one is replaced by its numeric form.
    let counts: Vec<Option<&Value>> = COUNTS.iter().map(|name| encoded.get(name)).collect();
    let mut numeric = encoded.clone();
    if let Some(field) = numeric.get_mut("max_in_flight_requests") {
        *field = json!(64);
    }
    // Then every count is a decimal string, the document round-trips, and a number fails.
    assert_eq!(
        counts,
        vec![
            Some(&json!("10000")),
            Some(&json!("100000")),
            Some(&json!("64")),
            Some(&json!("32"))
        ]
    );
    assert_eq!(serde_json::from_value::<Limits>(encoded)?, Limits::v1());
    assert!(serde_json::from_value::<Limits>(numeric).is_err());
    assert_eq!(
        serde_json::from_value::<Count>(json!("18446744073709551615"))?.get(),
        u64::MAX
    );
    Ok(())
}

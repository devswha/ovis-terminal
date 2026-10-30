//! Behavior tests for canonical decimal-string protocol counters.

use ovis_terminal_protocol::rpc::DecimalU64;
use proptest::prelude::*;

#[test]
fn decimal_counter_round_trips_u64_max_exactly() {
    let encoded =
        serde_json::to_string(&DecimalU64::new(u64::MAX)).expect("counter should serialize");
    let decoded: DecimalU64 =
        serde_json::from_str(&encoded).expect("canonical string should parse");

    assert_eq!(encoded, "\"18446744073709551615\"");
    assert_eq!(decoded.get(), u64::MAX);
}

#[test]
fn decimal_counter_rejects_json_numbers_and_non_canonical_strings() {
    for input in [
        "3",
        "3.0",
        "-1",
        "\"03\"",
        "\"\"",
        "\"+3\"",
        "\"18446744073709551616\"",
    ] {
        assert!(
            serde_json::from_str::<DecimalU64>(input).is_err(),
            "{input} must not parse as a canonical counter"
        );
    }
}

proptest! {
    #[test]
    fn decimal_counter_round_trips_every_value(value in any::<u64>()) {
        let encoded = serde_json::to_string(&DecimalU64::new(value))?;
        let decoded: DecimalU64 = serde_json::from_str(&encoded)?;
        prop_assert_eq!(encoded, format!("\"{value}\""));
        prop_assert_eq!(decoded.get(), value);
    }
}

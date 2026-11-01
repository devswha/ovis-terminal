use serde_json::{Value, json};

use super::{identity, metrics, queue};
use crate::domain::health::{CensusState, ConsumerLoss, RecoveryState};
use crate::schema::methods::server::{ServerMetricsResult, ServerStatusResult};
use crate::schema::scalar::{Count, DurationMs, OpenEnum};

const MAX: &str = "18446744073709551615";

fn saturated() -> ServerMetricsResult {
    let all = ConsumerLoss {
        range: super::events(0, u64::MAX),
        resynced: true,
    };
    metrics(u64::MAX, u64::MAX, vec![queue(vec![all])])
}

fn running(census: Option<(Count, Count)>) -> ServerStatusResult {
    ServerStatusResult::running(
        identity(),
        (DurationMs::new(1), census),
        RecoveryState::derive(false, (0, 0), false),
    )
}

fn refused_at<T: serde::de::DeserializeOwned>(value: Value) -> String {
    match serde_path_to_error::deserialize::<_, T>(value) {
        Ok(_) => "accepted".to_owned(),
        Err(error) => error.path().to_string(),
    }
}

#[test]
fn metric_counters_round_trip_u64_max_as_decimal_strings() {
    let result = saturated();
    let encoded = serde_json::to_value(&result).expect("encodes");
    for path in ["/streams", "/subscriptions", "/consumers/0/resynced_events"] {
        assert_eq!(encoded.pointer(path), Some(&json!(MAX)), "{path}");
    }
    let decoded: ServerMetricsResult = serde_json::from_value(encoded).expect("decodes");
    assert_eq!(decoded, result);
}

#[test]
fn numeric_metric_counters_are_refused_at_their_exact_path() {
    let encoded = serde_json::to_value(saturated()).expect("encodes");
    for (pointer, path) in [
        ("/streams", "streams"),
        ("/subscriptions", "subscriptions"),
        (
            "/consumers/0/resynced_events",
            "consumers[0].resynced_events",
        ),
    ] {
        let mut numeric = encoded.clone();
        if let Some(slot) = numeric.pointer_mut(pointer) {
            *slot = json!(1);
        }
        assert_eq!(refused_at::<ServerMetricsResult>(numeric), path);
    }
}

#[test]
fn a_reported_census_round_trips_u64_max_and_numbers_are_refused() {
    let status = running(Some((Count::new(u64::MAX), Count::new(u64::MAX))));
    assert_eq!(status.census, OpenEnum::Known(CensusState::Reported));
    let encoded = serde_json::to_value(&status).expect("encodes");
    assert_eq!(encoded.pointer("/holder_count"), Some(&json!(MAX)));
    assert_eq!(encoded.pointer("/live_terminals"), Some(&json!(MAX)));
    let decoded: ServerStatusResult = serde_json::from_value(encoded.clone()).expect("decodes");
    assert_eq!(decoded, status);
    for path in ["holder_count", "live_terminals"] {
        let mut numeric = encoded.clone();
        if let Some(slot) = numeric.get_mut(path) {
            *slot = json!(0);
        }
        assert_eq!(refused_at::<ServerStatusResult>(numeric), path);
    }
}

#[test]
fn an_unpublished_census_is_not_reported_never_zero() {
    let encoded = serde_json::to_value(running(None)).expect("encodes");
    assert_eq!(encoded.pointer("/census"), Some(&json!("not_reported")));
    assert_eq!(encoded.pointer("/holder_count"), Some(&Value::Null));
    assert_eq!(encoded.pointer("/live_terminals"), Some(&Value::Null));
}

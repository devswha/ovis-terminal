use ovis_terminal_core::events::Cursor;
use ovis_terminal_core::subscriptions::{LossCause, LossRecord, Request, Selector, Start};
use ovis_terminal_protocol::schema::ids::SubscriptionId;
use ovis_terminal_protocol::schema::methods::events::EventsSubscribeParams;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use proptest::prelude::*;
use serde_json::{Value, json};

use super::fixture::{TestResult, identity, terminal};

fn record(seq: u64) -> LossRecord {
    LossRecord::new(
        SubscriptionId::new("sub-1".into()),
        EventSeq::new(seq),
        LossCause::QueueOverflow,
    )
}

fn decode(params: Value) -> Result<Request, Box<dyn std::error::Error>> {
    let params: EventsSubscribeParams = serde_json::from_value(params)?;
    Ok(Request::decode(&params, identity(1)?.machine_id())?)
}

fn kind(params: Value) -> Result<&'static str, Box<dyn std::error::Error>> {
    let params: EventsSubscribeParams = serde_json::from_value(params)?;
    match Request::decode(&params, identity(1)?.machine_id()) {
        Ok(request) => Err(format!("accepted {request:?}").into()),
        Err(error) => Ok(error.kind()),
    }
}

proptest! {
    #[test]
    fn loss_markers_round_trip_every_sequence_as_a_decimal_string(seq in any::<u64>()) {
        let encoded = serde_json::to_value(record(seq)).map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(encoded.get("from_seq"), Some(&json!(seq.to_string())));
        let decoded: LossRecord = serde_json::from_value(encoded).map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(decoded, record(seq));
    }
}

#[test]
fn loss_markers_reject_numeric_and_unknown_fields() -> TestResult {
    // Given the exact u64::MAX marker and two hostile encodings of it.
    let exact = json!({"subscription_id": "sub-1", "from_seq": "18446744073709551615", "cause": "queue_overflow"});
    let numeric = json!({"subscription_id": "sub-1", "from_seq": 5, "cause": "queue_overflow"});
    let extra =
        json!({"subscription_id": "sub-1", "from_seq": "5", "cause": "queue_overflow", "seq": "5"});
    // When each is decoded.
    let decoded = serde_json::from_value::<LossRecord>(exact)?;
    // Then only the canonical string form is accepted.
    assert_eq!(decoded, record(u64::MAX));
    assert!(serde_json::from_value::<LossRecord>(numeric).is_err());
    assert!(serde_json::from_value::<LossRecord>(extra).is_err());
    Ok(())
}

#[test]
fn subscribe_parameters_decode_to_one_start_and_selector() -> TestResult {
    // Given the three start forms of events.subscribe.
    let runtime = identity(1)?;
    let lifetime = runtime.runtime_id().to_string();
    let filter = json!({"terminals": ["terminal:1"], "types": ["terminal.updated"]});
    // When each is decoded for this machine.
    let resume = decode(
        json!({"filter": filter, "cursor": {"runtime_lifetime": lifetime, "seq": "18446744073709551615"}, "snapshot": true}),
    )?;
    let snapshot = decode(json!({"filter": {}, "snapshot": true}))?;
    let head = decode(json!({"filter": {}, "snapshot": false}))?;
    // Then a cursor always resumes, and the snapshot flag only matters without one.
    let cursor = Cursor::new(
        runtime.machine_id(),
        runtime.runtime_id(),
        EventSeq::new(u64::MAX),
    );
    assert_eq!(resume.start, Start::Resume(cursor));
    let selector = Selector::all()
        .terminals([terminal(1)?])
        .types(["terminal.updated".to_owned()]);
    assert_eq!(resume.selector, selector);
    assert_eq!((snapshot.start, head.start), (Start::Snapshot, Start::Head));
    assert_eq!(snapshot.selector, Selector::all());
    Ok(())
}

#[test]
fn unusable_filters_and_cursors_are_typed_rejections() -> TestResult {
    // Given selections this core cannot evaluate exactly and a malformed lifetime.
    let cases = [
        (
            json!({"filter": {"labels": {"role": "agent"}}, "snapshot": true}),
            "filter_invalid",
        ),
        (
            json!({"filter": {"children_of": ["terminal:1"]}, "snapshot": true}),
            "filter_invalid",
        ),
        (
            json!({"filter": {"terminals": []}, "snapshot": true}),
            "filter_invalid",
        ),
        (
            json!({"filter": {"terminals": ["pane:1"]}, "snapshot": true}),
            "filter_invalid",
        ),
        (
            json!({"filter": {"machines": [{"name": "laptop"}]}, "snapshot": true}),
            "filter_invalid",
        ),
        (
            json!({"filter": {}, "cursor": {"runtime_lifetime": "nope", "seq": "1"}, "snapshot": false}),
            "invalid_params",
        ),
    ];
    // When each is decoded, then each fails with its stable kind.
    for (params, expected) in cases {
        assert_eq!(kind(params.clone())?, expected, "{params}");
    }
    // And a numeric cursor sequence never reaches the decoder.
    let numeric = json!({"filter": {}, "cursor": {"runtime_lifetime": "runtime:1", "seq": 1}, "snapshot": false});
    assert!(serde_json::from_value::<EventsSubscribeParams>(numeric).is_err());
    Ok(())
}

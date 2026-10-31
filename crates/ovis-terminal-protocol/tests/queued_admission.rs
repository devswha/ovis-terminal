//! Queued-operation positions and exceeded bounds cross the wire as decimal strings.

use ovis_terminal_protocol::errors::{BoundaryError, ErrorData, ExceededBound, LimitName, OpId};
use ovis_terminal_protocol::schema::ids::MethodName;
use ovis_terminal_protocol::schema::methods::server::{OpRecord, OpState};
use ovis_terminal_protocol::schema::scalar::{Count, DurationMs, OpenEnum, QueuePosition};
use proptest::prelude::*;
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn queued(position: u64) -> OpRecord {
    OpRecord::queued(
        OpId::new("01800000-0000-7000-8000-000000000001".into()),
        MethodName::new("doc.set".into()),
        QueuePosition::new(position),
    )
}

proptest! {
    #[test]
    fn queued_records_round_trip_every_position(position in any::<u64>()) {
        // Given a queued record at an arbitrary position.
        let record = queued(position);
        // When it crosses JSON.
        let wire = serde_json::to_value(&record).map_err(|error| TestCaseError::fail(error.to_string()))?;
        let decoded: OpRecord = serde_json::from_value(wire.clone()).map_err(|error| TestCaseError::fail(error.to_string()))?;
        // Then the state is queued and the position is an exact decimal string.
        prop_assert_eq!(wire.get("state"), Some(&json!("queued")));
        prop_assert_eq!(wire.get("queued_position"), Some(&json!(position.to_string())));
        prop_assert_eq!(decoded, record);
    }
}

#[test]
fn the_maximum_position_round_trips_and_a_number_is_refused() -> TestResult {
    // Given a deadline failure for a request still queued at the maximum position.
    let expired = BoundaryError::deadline_exceeded(
        OpId::new("01800000-0000-7000-8000-000000000001".into()),
        DurationMs::new(10_000),
        DurationMs::new(10_001),
    )
    .with_queued_position(QueuePosition::new(u64::MAX));
    let wire = serde_json::to_value(expired.data())?;
    // When it is decoded, and again with the position as a JSON number.
    let decoded: ErrorData = serde_json::from_value(wire.clone())?;
    let mut numeric = wire.clone();
    *numeric.get_mut("queued_position").ok_or("position")? = json!(u64::MAX);
    // Then the exact value survives and the numeric form is rejected.
    assert_eq!(
        wire.get("queued_position"),
        Some(&json!(u64::MAX.to_string()))
    );
    assert_eq!(decoded.queued_position, Some(QueuePosition::new(u64::MAX)));
    assert!(serde_json::from_value::<ErrorData>(numeric).is_err());
    let record: Value = serde_json::to_value(queued(u64::MAX))?;
    let mut numeric_record = record;
    *numeric_record
        .get_mut("queued_position")
        .ok_or("position")? = json!(1);
    assert!(serde_json::from_value::<OpRecord>(numeric_record).is_err());
    Ok(())
}

#[test]
fn an_exceeded_bound_names_the_limit_and_its_decimal_value() -> TestResult {
    // Given a refusal that reached an advertised limit.
    let refused = BoundaryError::domain("limit_exceeded".to_owned().into(), true).with_bound(
        ExceededBound::new(
            LimitName::new("queued_operations".into()),
            Count::new(u64::MAX),
        ),
    );
    // When its data crosses JSON.
    let wire = serde_json::to_value(refused.data())?;
    let decoded: ErrorData = serde_json::from_value(wire.clone())?;
    // Then the limit's name and exact value are both carried, and nothing else is invented.
    assert_eq!(
        wire.get("bound"),
        Some(&json!({"limit": "queued_operations", "value": u64::MAX.to_string()}))
    );
    assert_eq!(decoded, refused.data().clone());
    assert_eq!(wire.get("queued_position"), None);
    let mut numeric = wire;
    *numeric.pointer_mut("/bound/value").ok_or("value")? = json!(32);
    assert!(serde_json::from_value::<ErrorData>(numeric).is_err());
    Ok(())
}

#[test]
fn records_written_before_queueing_existed_still_decode() -> TestResult {
    // Given a retained record serialized without the additive position field.
    let legacy = json!({
        "op_id": "01800000-0000-7000-8000-000000000001", "method": "doc.set",
        "state": "committed", "result": null, "error": null, "updated_seq": "7"
    });
    // When it is decoded by this version.
    let record: OpRecord = serde_json::from_value(legacy.clone())?;
    // Then it is unchanged and re-encodes byte-for-byte without a position.
    assert_eq!(record.state, OpenEnum::Known(OpState::Committed));
    assert_eq!(record.queued_position, None);
    assert_eq!(serde_json::to_value(record)?, legacy);
    Ok(())
}

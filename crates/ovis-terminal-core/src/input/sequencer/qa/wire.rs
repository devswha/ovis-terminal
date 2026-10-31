//! Receipts on the wire: every counter is a canonical decimal string.

use ovis_terminal_core::input::receipts::{Delivery, FenceReason, InputSeq, Receipt};
use ovis_terminal_core::input::sequencer::{Payload, QueueLimits};
use ovis_terminal_protocol::schema::methods::input::Submit;
use proptest::prelude::{ProptestConfig, any, prop_assert_eq, proptest};
use serde_json::{Value, json};

use super::TestResult;
use super::fixture::{Fixture, WRITER, ack, admit, modes, submission, view};

fn written_receipt() -> TestResult<Receipt> {
    let fixture = Fixture::new()?;
    let (leases, modes) = (fixture.leases()?, modes(b"")?);
    let view = view(&leases, &modes);
    let alice = fixture.client("alice", WRITER)?;
    let mut sequencer = fixture.sequencer(QueueLimits::DEFAULT)?;
    let payload = Payload::Text {
        text: "ok",
        submit: &Submit::Enter,
    };
    admit(
        &mut sequencer,
        &view,
        (
            alice.writer(),
            submission("op-wire", fixture.expected()?, payload),
        ),
    )?;
    let input = sequencer.next(&view)?.dispatch.ok_or("no write")?;
    Ok(sequencer
        .acknowledge(&ack(&input, 3))?
        .ok_or("no receipt")?)
}

#[test]
fn receipts_carry_counters_as_decimal_strings_and_round_trip() -> TestResult {
    // Given a receipt of a fully written entry.
    let receipt = written_receipt()?;

    // When it is serialized and parsed back.
    let wire = serde_json::to_value(&receipt)?;
    let parsed: Receipt = serde_json::from_value(wire.clone())?;

    // Then counters are strings, stages are tagged and nothing is lost.
    assert_eq!(parsed, receipt);
    assert_eq!(wire.get("queued_seq"), Some(&json!("1")));
    assert_eq!(wire.get("admitted_bytes"), Some(&json!("3")));
    assert_eq!(
        wire.get("delivery"),
        Some(&json!({"state": "pty_written", "written_bytes": "3", "output_offset": "0"}))
    );
    assert_eq!(
        wire.get("agent_accepted"),
        Some(&json!({"state": "unknown"}))
    );
    Ok(())
}

#[test]
fn numeric_counters_and_unknown_fields_are_rejected() -> TestResult {
    // Given a valid receipt on the wire.
    let wire = serde_json::to_value(written_receipt()?)?;

    // When a counter is numeric or an unknown field is added.
    let with = |field: &str, value: Value| {
        let mut changed = wire.clone();
        changed
            .as_object_mut()
            .map(|object| object.insert(field.to_owned(), value));
        changed
    };
    let numeric = with("queued_seq", json!(1));
    let extra = with("force", Value::Bool(true));
    let non_canonical = with("queued_seq", json!("01"));

    // Then each is refused.
    for refused in [numeric, extra, non_canonical] {
        assert!(serde_json::from_value::<Receipt>(refused).is_err());
    }
    Ok(())
}

#[test]
fn input_sequence_round_trips_the_largest_value_exactly() -> TestResult {
    // Given the largest input sequence.
    let max = InputSeq::new(u64::MAX);

    // When it is serialized and parsed.
    let wire = serde_json::to_string(&max)?;

    // Then it is the exact decimal string.
    assert_eq!(wire, "\"18446744073709551615\"");
    assert_eq!(serde_json::from_str::<InputSeq>(&wire)?, max);
    Ok(())
}

#[test]
fn fence_reasons_publish_their_kind_token() -> TestResult {
    // Given fenced deliveries for every revocation kind.
    for reason in [
        FenceReason::StaleTarget,
        FenceReason::CredentialRevoked,
        FenceReason::LeaseRevoked,
        FenceReason::Unwritten,
    ] {
        // When serialized.
        let wire = serde_json::to_value(Delivery::Fenced { reason })?;
        // Then the reason token is the published kind.
        assert_eq!(wire, json!({"state": "fenced", "reason": reason.kind()}));
        assert_eq!(
            serde_json::from_value::<Delivery>(wire)?,
            Delivery::Fenced { reason }
        );
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn every_input_sequence_round_trips(value in any::<u64>()) {
        let seq = InputSeq::new(value);
        let wire = serde_json::to_value(seq).map_err(|error| proptest::test_runner::TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(&wire, &json!(value.to_string()));
        let parsed: InputSeq = serde_json::from_value(wire).map_err(|error| proptest::test_runner::TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(parsed, seq);
    }
}

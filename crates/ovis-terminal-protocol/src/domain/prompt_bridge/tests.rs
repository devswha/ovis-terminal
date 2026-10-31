use std::collections::BTreeMap;
use std::error::Error;

use proptest::prelude::*;
use serde_json::{Value, json};

use super::{DeliveryRecord, DeliveryState, ExtensionEvent, ProgramCredential};
use crate::domain::prompts::{AnswerDelivery, PromptAnswerReceipt, PromptOwner};
use crate::errors::OpId;
use crate::schema::ids::{PromptId, TerminalId};
use crate::schema::methods::status::{
    PromptAnswer, PromptDefinition, PromptKind, PromptMechanism, PromptRecord, PromptState,
};
use crate::schema::scalar::{EventSeq, Incarnation, OpenEnum};

type TestResult = Result<(), Box<dyn Error>>;

fn answered(mechanism: PromptMechanism, delivery: AnswerDelivery) -> PromptRecord {
    let owner = PromptOwner::new(
        TerminalId::new("terminal:4".to_owned()),
        Incarnation::new(7),
    );
    let definition = PromptDefinition::new(PromptKind::Form, "Release?".to_owned(), mechanism);
    let mut record = PromptRecord::opened(
        (PromptId::new("form-1".to_owned()), owner),
        definition,
        EventSeq::new(3),
    );
    let fields = BTreeMap::from([("branch".to_owned(), "main".to_owned())]);
    let given = (
        OpId::new("answer-op".to_owned()),
        PromptAnswer::Fields(fields),
    );
    record.close(PromptState::Answered, EventSeq::new(9));
    record.answer = Some(PromptAnswerReceipt::new(given, delivery, EventSeq::new(9)));
    record
}

#[test]
fn only_an_extension_answer_becomes_an_extension_event() -> TestResult {
    let record = answered(PromptMechanism::Extension, AnswerDelivery::Extension);
    let event = ExtensionEvent::from_answered(&record).ok_or("no event")?;
    assert_eq!(event.version, record.version);
    assert_eq!(event.version.get(), 2);
    assert_eq!(event.answer_op_id.as_str(), "answer-op");
    assert_eq!(event.answered_seq.get(), 9);
    assert_eq!(event.process_incarnation.get(), 7);
    let typed = answered(PromptMechanism::Keys, AnswerDelivery::Typed);
    let by_rpc = answered(PromptMechanism::Rpc, AnswerDelivery::Event);
    let mut dismissed = answered(PromptMechanism::Extension, AnswerDelivery::Extension);
    dismissed.state = OpenEnum::Known(PromptState::Dismissed);
    let mut open = answered(PromptMechanism::Extension, AnswerDelivery::Extension);
    open.answer = None;
    for record in [typed, by_rpc, dismissed, open] {
        assert_eq!(ExtensionEvent::from_answered(&record), None);
    }
    Ok(())
}

#[test]
fn a_credential_is_read_from_any_request_shape_but_needs_a_token() -> TestResult {
    let injected = json!({"process_token": "other-process", "op_id": "o",
        "prompt": {"kind": "form", "mechanism": "extension"}});
    let credential: ProgramCredential = serde_json::from_value(injected)?;
    assert_eq!(credential.process_token.as_str(), "other-process");
    assert_eq!((credential.target, credential.prompt_id), (None, None));
    let close = json!({"process_token": "t", "prompt_id": "p1", "outcome": "dismissed"});
    let credential: ProgramCredential = serde_json::from_value(close)?;
    assert_eq!(credential.prompt_id, Some(PromptId::new("p1".to_owned())));
    let tokenless = json!({"prompt": {"kind": "form"}});
    assert!(serde_json::from_value::<ProgramCredential>(tokenless).is_err());
    assert!(serde_json::from_value::<ProgramCredential>(json!({"process_token": 7})).is_err());
    Ok(())
}

#[test]
fn counters_are_decimal_strings_and_numbers_are_refused() -> TestResult {
    let mut record = answered(PromptMechanism::Extension, AnswerDelivery::Extension);
    record.version = crate::schema::scalar::RecordVersion::new(u64::MAX);
    let event = ExtensionEvent::from_answered(&record).ok_or("no event")?;
    let wire = serde_json::to_value(&event)?;
    assert_eq!(wire.get("version"), Some(&json!(u64::MAX.to_string())));
    assert_eq!(
        serde_json::from_value::<ExtensionEvent>(wire.clone())?,
        event
    );
    let mut numeric = wire;
    *numeric.pointer_mut("/version").ok_or("no version")? = json!(2);
    assert!(serde_json::from_value::<ExtensionEvent>(numeric).is_err());
    let delivery =
        serde_json::to_value(DeliveryRecord::new(&event, DeliveryState::OutcomeUnknown))?;
    assert_eq!(delivery.get("state"), Some(&json!("outcome_unknown")));
    assert_eq!(delivery.get("version"), Some(&json!(u64::MAX.to_string())));
    Ok(())
}

proptest! {
    #[test]
    fn extension_events_round_trip_every_counter(
        version in any::<u64>(),
        seq in any::<u64>(),
        incarnation in any::<u64>(),
        text in ".{1,32}",
    ) {
        let mut record = answered(PromptMechanism::Extension, AnswerDelivery::Extension);
        record.version = crate::schema::scalar::RecordVersion::new(version);
        record.process_incarnation = Incarnation::new(incarnation);
        let given = (OpId::new("op".to_owned()), PromptAnswer::Text(text));
        record.answer = Some(PromptAnswerReceipt::new(
            given,
            AnswerDelivery::Extension,
            EventSeq::new(seq),
        ));
        let event = ExtensionEvent::from_answered(&record);
        prop_assert!(event.is_some());
        let wire = serde_json::to_value(&event)?;
        prop_assert_eq!(wire.pointer("/version"), Some(&Value::from(version.to_string())));
        prop_assert_eq!(wire.pointer("/answered_seq"), Some(&Value::from(seq.to_string())));
        let incarnation = Value::from(incarnation.to_string());
        prop_assert_eq!(wire.pointer("/process_incarnation"), Some(&incarnation));
        let back: Option<ExtensionEvent> = serde_json::from_value(wire)?;
        prop_assert_eq!(back, event);
    }
}

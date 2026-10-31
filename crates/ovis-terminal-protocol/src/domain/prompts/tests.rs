use proptest::prelude::*;
use serde_json::{Value, json};

use super::{AnswerDelivery, PromptAnswerReceipt, PromptOwner};
use crate::errors::{BoundaryError, ErrorData, ErrorKind, OpId};
use crate::schema::ids::{PromptId, TerminalId};
use crate::schema::methods::status::{
    PromptAnswer, PromptAnswerParams, PromptChoice, PromptDefinition, PromptKind, PromptMechanism,
    PromptOpenParams, PromptRecord, PromptState,
};
use crate::schema::scalar::{EventSeq, Incarnation, OpenEnum, RecordVersion};

fn fail(error: impl std::fmt::Display) -> TestCaseError {
    TestCaseError::fail(error.to_string())
}

#[test]
fn a_version_conflict_carries_current_version_as_an_exact_decimal_string()
-> Result<(), serde_json::Error> {
    let error = BoundaryError::domain(ErrorKind::from("version_conflict".to_owned()), false)
        .with_current_version(RecordVersion::new(u64::MAX))
        .with_prompt_ids(vec![PromptId::new("p1".to_owned())]);
    let wire = serde_json::to_value(error.data())?;
    assert_eq!(
        wire.get("current_version"),
        Some(&json!("18446744073709551615"))
    );
    assert_eq!(wire.get("prompt_ids"), Some(&json!(["p1"])));
    let back: ErrorData = serde_json::from_value(wire)?;
    assert_eq!(back.current_version, Some(RecordVersion::new(u64::MAX)));
    let numeric = json!({"kind": "version_conflict", "hint": "h", "retryable": false,
                         "current_version": 2});
    assert!(serde_json::from_value::<ErrorData>(numeric).is_err());
    Ok(())
}

#[test]
fn an_answer_names_its_version_as_a_decimal_string_and_its_answer_as_a_tagged_value() {
    let params = |version: Value, answer: Value| json!({"op_id": "o", "prompt_id": "p1", "version": version, "answer": answer});
    let parsed =
        serde_json::from_value::<PromptAnswerParams>(params(json!("2"), json!({"text": "yes"})));
    assert!(
        parsed.is_ok_and(|params| params.version == RecordVersion::new(2)
            && params.answer == PromptAnswer::Text("yes".to_owned()))
    );
    for (version, answer) in [
        (json!(1), json!({"text": "yes"})),
        (json!("1"), json!("yes")),
        (json!("1"), json!({"text": "yes", "force": true})),
    ] {
        assert!(serde_json::from_value::<PromptAnswerParams>(params(version, answer)).is_err());
    }
}

#[test]
fn an_open_request_may_name_its_prompt_and_rejects_unknown_fields() {
    let open = |extra: Value| {
        let mut params = json!({"op_id": "o", "target": {"name": "agent"}, "process_token": "t",
            "prompt": {"kind": "question", "text": "q", "choices": [], "mechanism": "rpc"}});
        if let (Some(params), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
            params.extend(extra.clone());
        }
        serde_json::from_value::<PromptOpenParams>(params)
    };
    assert!(open(json!({})).is_ok_and(|params| params.prompt_id.is_none()));
    assert!(
        open(json!({"prompt_id": "p1"}))
            .is_ok_and(|params| params.prompt_id == Some(PromptId::new("p1".to_owned())))
    );
    assert!(open(json!({"force": true})).is_err());
}

#[test]
fn republish_and_close_each_advance_the_version_once() {
    let owner = PromptOwner::new(
        TerminalId::new("terminal:4".to_owned()),
        Incarnation::new(1),
    );
    let definition =
        || PromptDefinition::new(PromptKind::Question, "q".to_owned(), PromptMechanism::Rpc);
    let mut record = PromptRecord::opened(
        (PromptId::new("p1".to_owned()), owner),
        definition(),
        EventSeq::new(3),
    );
    assert_eq!(
        (record.version, record.is_open()),
        (RecordVersion::new(1), true)
    );
    record.republish(definition());
    assert_eq!(record.version, RecordVersion::new(2));
    record.close(PromptState::Answered, EventSeq::new(9));
    assert_eq!(record.version, RecordVersion::new(3));
    assert_eq!(
        (record.is_open(), record.closed_seq),
        (false, Some(EventSeq::new(9)))
    );
}

proptest! {
    #[test]
    fn prompt_records_round_trip_with_decimal_counters(
        version in any::<u64>(), opened in any::<u64>(), closed in any::<u64>(),
        text in ".{0,64}", choice in "[a-z]{1,8}", answer in ".{0,64}",
    ) {
        let owner = PromptOwner::new(TerminalId::new("terminal:4".to_owned()), Incarnation::new(u64::MAX));
        let mut definition = PromptDefinition::new(PromptKind::Permission, text, PromptMechanism::Keys);
        definition.choices.push(PromptChoice::new(choice.clone(), choice));
        let mut record = PromptRecord::opened((PromptId::new("p".to_owned()), owner), definition, EventSeq::new(opened));
        record.version = RecordVersion::new(version);
        record.close(PromptState::Answered, EventSeq::new(closed));
        let receipt = PromptAnswerReceipt::new(
            (OpId::new("o".to_owned()), PromptAnswer::Text(answer)), AnswerDelivery::Typed, EventSeq::new(closed));
        record.answer = Some(receipt);
        let wire = serde_json::to_value(&record).map_err(fail)?;
        prop_assert_eq!(wire.get("opened_seq"), Some(&json!(opened.to_string())));
        prop_assert_eq!(wire.get("process_incarnation"), Some(&json!(u64::MAX.to_string())));
        let back: PromptRecord = serde_json::from_value(wire).map_err(fail)?;
        prop_assert_eq!(&back.state, &OpenEnum::Known(PromptState::Answered));
        prop_assert_eq!(back, record);
    }
}

use proptest::prelude::*;
use serde_json::{Value, json};

use super::{
    AgentRecordsParams, AppendRecordsParams, ContentBlock, ConversationSource, ConversationTurn,
    MessagePhase, RecordBlock, RecordInput, RecordRole, TurnState, Unavailable, UnavailableReason,
};
use crate::errors::{BoundaryError, ErrorData, ErrorKind};
use crate::schema::ids::{AgentId, CallId, RecordId};
use crate::schema::methods::agent::{AgentConversationParams, ConversationRecord};
use crate::schema::scalar::{EventSeq, OpenEnum, TimestampMs, TurnNumber};

fn append(records: &Value) -> Value {
    json!({"op_id": "o", "target": {"name": "agent"}, "process_token": "t", "records": records})
}

fn refused_at<T: serde::de::DeserializeOwned>(value: Value) -> Result<String, String> {
    serde_path_to_error::deserialize::<_, T>(value)
        .err()
        .map(|error| error.path().to_string())
        .ok_or_else(|| "the value decoded".to_owned())
}

#[test]
fn a_user_record_keeps_text_and_tool_result_blocks_in_provider_order() -> Result<(), String> {
    let wire = append(&json!([{
        "record_id": "u2", "role": "user", "blocks": [
            {"type": "text", "text": "here is the file"},
            {"type": "tool_result", "call_id": "c1", "output": "fn main() {}", "is_error": false},
            {"type": "text", "text": "and more"}
        ]
    }]));
    let params: AppendRecordsParams = serde_json::from_value(wire).map_err(|e| e.to_string())?;
    let record = params.records.first().ok_or("no record")?;
    assert_eq!(record.role, RecordRole::User);
    assert_eq!(
        record.blocks,
        vec![
            ContentBlock::Text {
                text: "here is the file".to_owned()
            },
            ContentBlock::ToolResult {
                call_id: CallId::new("c1".to_owned()),
                output: "fn main() {}".to_owned(),
                is_error: false
            },
            ContentBlock::Text {
                text: "and more".to_owned()
            },
        ]
    );
    Ok(())
}

#[test]
fn unknown_block_types_fields_roles_and_numeric_times_are_refused_at_their_path()
-> Result<(), String> {
    let record = |extra: Value| {
        let mut base = json!({"record_id": "r", "role": "assistant",
            "blocks": [{"type": "text", "text": "x"}]});
        if let (Some(base), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
            base.extend(extra.clone());
        }
        append(&json!([base]))
    };
    let cases = [
        (record(json!({"role": "system"})), "records[0].role"),
        (record(json!({"phase": "draft"})), "records[0].phase"),
        (record(json!({"at_ms": 5})), "records[0].at_ms"),
        (
            record(json!({"blocks": [{"type": "image", "text": "x"}]})),
            "records[0].blocks[0].type",
        ),
        (
            record(json!({"blocks": [{"type": "text", "text": "x", "screen": true}]})),
            "records[0].blocks[0]",
        ),
    ];
    for (wire, path) in cases {
        assert_eq!(refused_at::<AppendRecordsParams>(wire)?, path);
    }
    Ok(())
}

#[test]
fn no_conversation_read_takes_a_source_so_terminal_screen_is_refused_at_source()
-> Result<(), String> {
    let screen = json!({"target": {"name": "agent"}, "source": "terminal_screen"});
    assert_eq!(
        refused_at::<AgentConversationParams>(screen.clone())?,
        "source"
    );
    assert_eq!(refused_at::<AgentRecordsParams>(screen)?, "source");
    let records: AgentRecordsParams = serde_json::from_value(json!({"target": {"name": "agent"},
        "cursor": {"history_id": "h", "revision": "1", "position": "2"}}))
    .map_err(|e| e.to_string())?;
    assert!(records.cursor.is_some());
    Ok(())
}

#[test]
fn a_transcript_block_outside_the_contract_round_trips_unchanged() -> Result<(), serde_json::Error>
{
    let provider = json!({"text": "t-r1", "cache": {"hit": true}});
    let typed = json!({"type": "tool_call", "call_id": "c1", "name": "read", "input": {"p": 1}});
    let blocks: Vec<RecordBlock> = serde_json::from_value(json!([provider, typed]))?;
    assert!(matches!(blocks.first(), Some(RecordBlock::Provider(_))));
    assert!(matches!(
        blocks.get(1),
        Some(RecordBlock::Typed(ContentBlock::ToolCall { .. }))
    ));
    assert_eq!(serde_json::to_value(&blocks)?, json!([provider, typed]));
    Ok(())
}

#[test]
fn unavailable_names_the_reason_and_what_was_consulted() -> Result<(), serde_json::Error> {
    let unavailable = Unavailable::new(
        UnavailableReason::NotReported,
        ConversationSource::Records,
        (Some(AgentId::new("agent:1".to_owned())), None),
    );
    let error = BoundaryError::domain(
        ErrorKind::from("conversation_unavailable".to_owned()),
        false,
    )
    .with_unavailable(unavailable.clone());
    let wire = serde_json::to_value(error.data())?;
    assert_eq!(
        wire.get("unavailable"),
        Some(&json!({"reason": "not_reported",
            "provenance": {"source": "records", "agent_id": "agent:1", "session": null}}))
    );
    let back: ErrorData = serde_json::from_value(wire)?;
    assert_eq!(back.unavailable.as_deref(), Some(&unavailable));
    Ok(())
}

#[test]
fn an_open_turn_serializes_every_unknown_fact_as_null() -> Result<(), serde_json::Error> {
    let turn = ConversationTurn::new(TurnNumber::new(1), TurnState::Open);
    assert_eq!(
        serde_json::to_value(&turn)?,
        json!({"turn": "1", "state": "open", "outcome": null, "started_seq": null,
            "ended_seq": null, "started_ms": null, "ended_ms": null, "model": null,
            "reasoning": null})
    );
    Ok(())
}

fn record_with(turn: u64, at: u64) -> ConversationRecord {
    let mut record = ConversationRecord::new(
        RecordId::new("a1".to_owned()),
        None,
        (
            "assistant".to_owned(),
            vec![RecordBlock::Typed(ContentBlock::Text {
                text: "x".to_owned(),
            })],
        ),
    );
    record.phase = Some(OpenEnum::Known(MessagePhase::Commentary));
    record.turn = Some(TurnNumber::new(turn));
    record.at_ms = Some(TimestampMs::new(at));
    record
}

#[test]
fn u64_max_counters_round_trip_as_exact_decimal_strings() -> Result<(), serde_json::Error> {
    let record = record_with(u64::MAX, u64::MAX);
    let wire = serde_json::to_value(&record)?;
    assert_eq!(wire.get("turn"), Some(&json!("18446744073709551615")));
    assert_eq!(wire.get("at_ms"), Some(&json!("18446744073709551615")));
    assert_eq!(serde_json::from_value::<ConversationRecord>(wire)?, record);
    let input: RecordInput = serde_json::from_value(json!({"record_id": "r", "role": "tool",
        "blocks": [], "at_ms": "18446744073709551615"}))?;
    assert_eq!(input.at_ms, Some(TimestampMs::new(u64::MAX)));
    Ok(())
}

proptest! {
    #[test]
    fn records_and_turns_round_trip_for_any_counter(turn in any::<u64>(), at in any::<u64>()) {
        let record = record_with(turn, at);
        let text = serde_json::to_string(&record).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let back: ConversationRecord =
            serde_json::from_str(&text).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, record);
        let mut open = ConversationTurn::new(TurnNumber::new(turn), TurnState::Ended);
        open.started_seq = Some(EventSeq::new(at));
        let text = serde_json::to_string(&open).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let back: ConversationTurn =
            serde_json::from_str(&text).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, open);
    }
}

//! Appended records keep order, identities, phases, mixed blocks and call links; the event
//! and the receipt name exactly the records every later read returns.

use ovis_terminal_protocol::domain::conversation_records::{
    ContentBlock, MessagePhase, RecordBlock, ToolLink, TurnState,
};
use ovis_terminal_protocol::schema::ids::{CallId, RecordId};
use ovis_terminal_protocol::schema::scalar::{HistoryRevision, OpenEnum, TurnNumber};

use super::*;

#[test]
fn mixed_records_link_results_to_calls_and_the_event_matches_every_read() -> TestResult {
    // Given: a subscriber, a registered agent and a running status turn.
    let h = harness()?;
    let events = h.core.subscribe(None)?;
    h.register()?;
    h.report(AgentState::Working, (1, 1_000))?;

    // When: the owner appends the whole conversation in one append.
    let appended: RecordsAppended = result(&h.append(&conversation())?)?;

    // Then: the event equals the receipt, and both name exactly the records a read returns.
    let event = next_appended(&events)?;
    assert_eq!(event, appended);
    let page = h.page()?;
    let read: Vec<RecordId> = page.records.iter().map(|r| r.record_id.clone()).collect();
    assert_eq!(event.record_ids, read);
    assert_eq!(event.digest.as_str(), digest_of(&page.records)?);
    assert_eq!(event.first_position.get(), 0);
    assert_eq!(event.total_records.get(), 4);
    assert_eq!(page.history_id, event.history_id);
    assert_eq!(page.revision, HistoryRevision::new(1));
    assert_eq!(ids(&page), ["u1", "a1", "u2", "a2"]);
    let parents: Vec<_> = page.records.iter().map(|r| r.parent.clone()).collect();
    let record = |id: &str| Some(RecordId::new(id.to_owned()));
    assert_eq!(parents, [None, record("u1"), record("a1"), record("u2")]);

    // The user's text and tool result share one record, in provider order, linked to a1.
    let user = page.records.get(2).ok_or("u2")?;
    assert_eq!(user.role, "user");
    assert!(matches!(
        user.blocks.as_slice(),
        [
            RecordBlock::Typed(ContentBlock::Text { .. }),
            RecordBlock::Typed(ContentBlock::ToolResult { .. })
        ]
    ));
    assert_eq!(
        user.links,
        [ToolLink::new(
            CallId::new("c1".to_owned()),
            RecordId::new("a1".to_owned())
        )]
    );
    assert_eq!(user.at_ms.map(TimestampMs::get), Some(1_700_000_000_123));

    // Commentary stays commentary; only the record the provider marked final is final.
    let phases: Vec<_> = page.records.iter().map(|r| r.phase.clone()).collect();
    let phase = |phase| Some(OpenEnum::Known(phase));
    assert_eq!(
        phases,
        [
            None,
            phase(MessagePhase::Commentary),
            None,
            phase(MessagePhase::Final)
        ]
    );
    assert!(
        page.records
            .iter()
            .all(|r| r.turn == Some(TurnNumber::new(1)))
    );
    let [turn] = page.turns.as_slice() else {
        return Err(format!("expected one turn, got {:?}", page.turns).into());
    };
    assert_eq!(turn.state, OpenEnum::Known(TurnState::Open));
    assert_eq!(turn.started_ms, Some(TimestampMs::new(1_000)));
    assert_eq!((turn.ended_ms, turn.outcome.clone()), (None, None));
    Ok(())
}

#[test]
fn records_reported_while_idle_belong_to_no_turn() -> TestResult {
    let h = harness()?;
    h.register()?;

    let appended: RecordsAppended = result(&h.append(&json!([text("u1", "user", "hi")]))?)?;

    let page = h.page()?;
    let record = page.records.first().ok_or("u1")?;
    assert_eq!(record.turn, None);
    assert_eq!(appended.record_ids, [RecordId::new("u1".to_owned())]);
    assert_eq!(appended.turns, []);
    assert_eq!(page.turns, []);
    Ok(())
}

#[test]
fn a_later_append_continues_the_chain_and_links_a_call_made_earlier() -> TestResult {
    let h = harness()?;
    h.register()?;
    let call = json!([{"record_id": "a1", "role": "assistant", "blocks": [
        {"type": "tool_call", "call_id": "c9", "name": "grep", "input": null}]}]);
    h.append(&call)?;

    let result_record = json!([{"record_id": "t1", "role": "tool", "blocks": [
        {"type": "tool_result", "call_id": "c9", "output": "no match", "is_error": true}]}]);
    let appended: RecordsAppended = result(&h.append(&result_record)?)?;

    assert_eq!(appended.revision, HistoryRevision::new(2));
    assert_eq!(appended.first_position.get(), 1);
    let page = h.page()?;
    let tool = page.records.get(1).ok_or("t1")?;
    assert_eq!(tool.parent, Some(RecordId::new("a1".to_owned())));
    assert_eq!(
        tool.links,
        [ToolLink::new(
            CallId::new("c9".to_owned()),
            RecordId::new("a1".to_owned())
        )]
    );
    assert_eq!(ids(&page), ["a1", "t1"]);
    Ok(())
}

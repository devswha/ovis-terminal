//! Turn records follow the status turn: its boundaries, outcome and times, nothing
//! guessed from later activity.

use ovis_terminal_protocol::domain::conversation_records::{ConversationTurn, TurnState};
use ovis_terminal_protocol::schema::methods::status::TurnOutcome;
use ovis_terminal_protocol::schema::scalar::{OpenEnum, TurnNumber};

use super::*;

fn only_turn(page: &AgentConversationResult) -> Result<&ConversationTurn, Box<dyn Error>> {
    match page.turns.as_slice() {
        [turn] => Ok(turn),
        turns => Err(format!("expected one turn, got {turns:?}").into()),
    }
}

#[test]
fn an_ended_turn_takes_outcome_and_end_time_from_the_report_that_ended_it() -> TestResult {
    // Given: turn 1 started at 1000 with records and the model recorded.
    let h = harness()?;
    h.register()?;
    h.report(AgentState::Working, (1, 1_000))?;
    let meta = serde_json::from_value(json!({"model": "model-a"}))?;
    h.append_as((TOKEN, &conversation()), Some(meta))?;

    // When: the root reports idle at 5000, and a read follows much later.
    h.report(AgentState::Idle, (2, 5_000))?;
    let page = h.page()?;

    // Then: the turn ended completed at 5000; reasoning was never recorded and stays null.
    let turn = only_turn(&page)?;
    assert_eq!(turn.turn, TurnNumber::new(1));
    assert_eq!(turn.state, OpenEnum::Known(TurnState::Ended));
    assert_eq!(turn.outcome, Some(OpenEnum::Known(TurnOutcome::Completed)));
    assert_eq!(turn.started_ms, Some(TimestampMs::new(1_000)));
    assert_eq!(turn.ended_ms, Some(TimestampMs::new(5_000)));
    assert!(turn.started_seq < turn.ended_seq);
    assert_eq!(turn.model.as_deref(), Some("model-a"));
    assert_eq!(turn.reasoning, None);
    Ok(())
}

#[test]
fn end_facts_overwritten_before_any_append_stay_null_instead_of_borrowing_a_later_time()
-> TestResult {
    // Given: turn 1 has records; it ends, and turn 2 runs and ends with no append between.
    let h = harness()?;
    h.register()?;
    h.report(AgentState::Working, (1, 1_000))?;
    h.append(&json!([text("u1", "user", "first")]))?;
    h.report(AgentState::Idle, (2, 2_000))?;
    h.report(AgentState::Working, (3, 9_000))?;
    h.report(AgentState::Done, (4, 9_500))?;

    let page = h.page()?;

    // Then: turn 1 is ended, but its outcome and end are no longer recorded anywhere.
    let turn = only_turn(&page)?;
    assert_eq!(turn.turn, TurnNumber::new(1));
    assert_eq!(turn.state, OpenEnum::Known(TurnState::Ended));
    assert_eq!((turn.outcome.clone(), turn.ended_ms), (None, None));
    assert_eq!(turn.started_ms, Some(TimestampMs::new(1_000)));
    Ok(())
}

#[test]
fn an_append_after_the_end_keeps_the_end_facts_once_status_moves_on() -> TestResult {
    let h = harness()?;
    h.register()?;
    h.report(AgentState::Working, (1, 1_000))?;
    h.append(&json!([text("u1", "user", "first")]))?;
    h.report(AgentState::Error, (2, 2_000))?;
    h.append(&json!([text("u2", "user", "second")]))?;
    h.report(AgentState::Working, (3, 9_000))?;
    h.report(AgentState::Idle, (4, 9_500))?;

    let page = h.page()?;

    let turn = only_turn(&page)?;
    assert_eq!(turn.outcome, Some(OpenEnum::Known(TurnOutcome::Failed)));
    assert_eq!(turn.ended_ms, Some(TimestampMs::new(2_000)));
    Ok(())
}

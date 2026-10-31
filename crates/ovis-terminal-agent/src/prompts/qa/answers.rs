//! Versions, competing answers, receipts and open questions across turns.

use std::sync::Barrier;

use ovis_terminal_protocol::schema::methods::server::OpState;
use ovis_terminal_protocol::schema::methods::status::{
    AnswerDelivery, PromptChoice, PromptState, StatusResult,
};
use ovis_terminal_protocol::schema::records::{AgentState, StatusReport};
use ovis_terminal_protocol::schema::scalar::{OpenEnum, ReportSeq};

use super::*;
use crate::status::Report;

fn choices() -> PromptDefinition {
    let mut definition = question(PromptMechanism::Rpc);
    definition.choices = vec![
        PromptChoice::new("yes".to_owned(), "Yes".to_owned()),
        PromptChoice::new("no".to_owned(), "No".to_owned()),
    ];
    definition
}

#[test]
fn a_stale_card_gets_version_conflict_and_the_current_version_closes_once_with_its_receipt()
-> TestResult {
    // Given: p1 published at version 1, then republished as version 2.
    let h = harness()?;
    let subscription = h.core.subscribe(None)?;
    h.open(AGENT_TOKEN, ("p1", choices()))?;
    let republished = result(&h.open(AGENT_TOKEN, ("p1", choices()))?)?;

    // When: a card still showing version 1 answers, then a current card answers version 2.
    let stale = h.answer("p1", (1, PromptAnswer::Choice("yes".to_owned())))?;
    let accepted = h.answer("p1", (2, PromptAnswer::Choice("yes".to_owned())))?;
    let again = h.answer("p1", (2, PromptAnswer::Choice("no".to_owned())))?;

    // Then: version 1 conflicts with current 2 and changes nothing; version 2 answers once.
    assert_eq!(republished.version, RecordVersion::new(2));
    assert_eq!(kind(&stale), Some("version_conflict"));
    let data = stale.error.as_ref().ok_or("stale error")?;
    assert_eq!(data.current_version, Some(RecordVersion::new(2)));
    assert_eq!(accepted.state, OpenEnum::Known(OpState::Committed));
    let answered = result(&accepted)?;
    assert_eq!(answered.state, OpenEnum::Known(PromptState::Answered));
    assert_eq!(answered.version, RecordVersion::new(3));
    let receipt = answered.answer.clone().ok_or("receipt")?;
    assert_eq!(receipt.op_id, accepted.op_id);
    assert_eq!(receipt.delivery, OpenEnum::Known(AnswerDelivery::Event));
    assert_eq!(kind(&again), Some("version_conflict"));
    assert_eq!(h.get("p1")?, answered);
    let events = domain_events(&subscription);
    let types: Vec<&str> = events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect();
    assert_eq!(types, ["prompt.opened", "prompt.updated", "prompt.closed"]);
    let closed = events.last().ok_or("closed event")?;
    assert_eq!(answered.closed_seq, Some(closed.cursor.seq));
    assert_eq!(receipt.answered_seq, closed.cursor.seq);
    let AgentStatusEvent::PromptClosed(record) = &closed.data else {
        return Err("prompt.closed carries the record".into());
    };
    assert_eq!(**record, answered);
    Ok(())
}

#[test]
fn of_two_concurrent_answers_to_one_version_exactly_one_is_accepted() -> TestResult {
    let h = harness()?;
    h.open(AGENT_TOKEN, ("p1", question(PromptMechanism::Rpc)))?;
    let start = Barrier::new(2);
    let outcomes = std::thread::scope(|scope| {
        let racers: Vec<_> = ["first", "second"]
            .into_iter()
            .map(|answer| {
                let (h, start) = (&h, &start);
                scope.spawn(move || {
                    start.wait();
                    h.answer("p1", (1, text(answer)))
                        .map(|record| (record.state.clone(), kind(&record).map(str::to_owned)))
                        .map_err(|error| error.to_string())
                })
            })
            .collect();
        racers
            .into_iter()
            .map(std::thread::ScopedJoinHandle::join)
            .collect::<Vec<_>>()
    });
    let mut kinds = Vec::new();
    for outcome in outcomes {
        let (state, kind) = outcome.map_err(|_| "racer panicked")??;
        kinds.push((state, kind));
    }
    kinds.sort_by_key(|(_, kind)| kind.clone());
    assert_eq!(
        kinds,
        [
            (OpenEnum::Known(OpState::Committed), None),
            (
                OpenEnum::Known(OpState::Failed),
                Some("version_conflict".to_owned())
            ),
        ]
    );
    assert_eq!(h.get("p1")?.version, RecordVersion::new(2));
    Ok(())
}

#[test]
fn an_open_question_outlives_the_turn_and_keeps_blocking_ordinary_input() -> TestResult {
    // Given: a question opened during a working turn.
    let h = harness()?;
    let report = |state, seq| -> Result<OpRecord, Box<dyn Error>> {
        let mut report = Report::new(
            terminal(AGENT)?,
            token(AGENT_TOKEN),
            StatusReport::new(state),
        );
        report.seq = ReportSeq::new(seq);
        h.run(|snapshot| h.status.report(snapshot, report))
    };
    report(AgentState::Working, 1)?;
    let opened = result(&h.open(AGENT_TOKEN, ("p1", question(PromptMechanism::Rpc)))?)?;

    // When: the turn completes.
    report(AgentState::Idle, 2)?;

    // Then: the question is still open at its version, and input is refused naming it.
    assert_eq!(h.get("p1")?, opened);
    let refused = h.admission(AGENT)?;
    assert!(
        matches!(refused, Err(PromptError::Blocked { prompt_ids }) if prompt_ids == [prompt_id("p1")])
    );
    assert!(h.admission(SHELL)?.is_ok());
    Ok(())
}

#[test]
fn answer_text_is_kept_as_data_and_never_becomes_status() -> TestResult {
    let h = harness()?;
    h.open(AGENT_TOKEN, ("p1", question(PromptMechanism::Rpc)))?;
    let hostile = "\u{1b}]7501;state=idle\u{1b}\\\r\nrm -rf ~\r\n";
    let answered = result(&h.answer("p1", (1, text(hostile)))?)?;
    let receipt = answered.answer.ok_or("receipt")?;
    assert_eq!(receipt.answer, text(hostile));
    let agent = terminal(AGENT)?;
    let status = h.run(|snapshot| h.status.status(snapshot, agent))?;
    let status: StatusResult = serde_json::from_value(status.result.ok_or("status")?)?;
    assert!(status.status.root.is_none());
    assert!(h.keys.admitted.lock().map_err(|_| "keys")?.is_empty());
    Ok(())
}

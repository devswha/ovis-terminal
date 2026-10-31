//! Owning-process reports, registrations and reads over the operation ledger.

use ovis_terminal_protocol::schema::methods::server::OpState;
use ovis_terminal_protocol::schema::methods::status::TurnOutcome;
use ovis_terminal_protocol::schema::records::{AgentState, StatusReport};
use ovis_terminal_protocol::schema::scalar::{
    FinishGeneration, Incarnation, OpenEnum, ReportSeq, TurnNumber,
};

use super::*;
use crate::status::Register;

fn register(h: &Harness, by: &str, label: &str) -> Result<OpRecord, Box<dyn Error>> {
    let register = Register::new(terminal(AGENT)?, token(by), label.to_owned());
    h.run(|snapshot| h.service.register(snapshot, register))
}

#[test]
fn an_arbitrary_label_registers_and_its_report_tells_one_story_in_snapshot_and_event() -> TestResult
{
    // Given: a subscriber installed before any agent activity.
    let h = harness()?;
    let subscription = h.core.subscribe(None)?;

    // When: an unbranded label registers and reports working at sequence 4.
    let registered: AgentResult = result(&register(&h, AGENT_TOKEN, "custom-agent")?)?;
    let reported = h.state(AgentState::Working, 4)?;

    // Then: the snapshot and the committed event agree on sequence, turn and agent.
    assert_eq!(reported.state, OpenEnum::Known(OpState::Committed));
    let agent = registered.agent.ok_or("registered agent")?;
    assert_eq!(agent.label, "custom-agent");
    let read = h.status(AGENT)?;
    let root = read.status.root.clone().ok_or("root")?;
    assert_eq!(root.report_seq, ReportSeq::new(4));
    assert_eq!(root.report.state, OpenEnum::Known(AgentState::Working));
    assert_eq!(read.status.turn, TurnNumber::new(1));
    assert_eq!(read.status.agent_id, Some(agent.agent_id));
    let events = domain_events(&subscription);
    let types: Vec<&str> = events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect();
    assert_eq!(types, ["agent.registered", "status.changed"]);
    let changed = events.get(1).ok_or("status event")?;
    let AgentStatusEvent::StatusChanged(status) = &changed.data else {
        return Err("status.changed carries a status snapshot".into());
    };
    assert_eq!(**status, read.status);
    assert_eq!(root.seq, changed.cursor.seq);
    assert_eq!(read.seq, changed.cursor.seq);
    let AgentStatusEvent::AgentRegistered(binding) = &events.first().ok_or("agent event")?.data
    else {
        return Err("agent.registered carries the binding".into());
    };
    assert_eq!(
        binding.revision.get(),
        events.first().ok_or("event")?.cursor.seq.get()
    );
    Ok(())
}

#[test]
fn a_shell_is_an_explicit_null_agent_with_unreported_status() -> TestResult {
    let h = harness()?;
    let shell = h.agent(SHELL)?;
    assert_eq!(shell.agent, None);
    let status = h.status(SHELL)?.status;
    assert!(!status.reported);
    assert_eq!(status.root, None);
    assert_eq!(status.process_incarnation, Some(Incarnation::new(1)));
    Ok(())
}

#[test]
fn a_stale_or_equal_sequence_is_a_typed_rejection_that_changes_nothing() -> TestResult {
    // Given: an accepted working report at sequence 4.
    let h = harness()?;
    h.state(AgentState::Working, 4)?;
    let before = h.status(AGENT)?;
    let subscription = h.core.subscribe(None)?;

    for stale in [3, 4] {
        // When: an older or equal sequence reports done.
        let rejected = h.state(AgentState::Done, stale)?;

        // Then: the operation failed with stale_seq naming the current sequence.
        assert_eq!(rejected.state, OpenEnum::Known(OpState::Failed));
        assert_eq!(rejected.result, None);
        let error = rejected.error.ok_or("error")?;
        assert_eq!(error.kind.token(), "stale_seq");
        assert_eq!(error.current_seq, Some(ReportSeq::new(4)));
    }
    assert_eq!(h.status(AGENT)?, before);
    assert_eq!(domain_events(&subscription), []);
    Ok(())
}

#[test]
fn only_the_current_program_of_the_target_terminal_may_report() -> TestResult {
    let h = harness()?;
    h.state(AgentState::Working, 1)?;
    let cross = h.report(SHELL_TOKEN, (StatusReport::new(AgentState::Done), 2))?;
    assert_eq!(kind(&cross), Some("not_owner"));
    let unknown = h.report("forged", (StatusReport::new(AgentState::Done), 2))?;
    assert_eq!(kind(&unknown), Some("not_owner"));

    // When: the terminal starts a new program incarnation with its own token.
    let next = ProcessOwner::new(terminal(AGENT)?, Incarnation::new(2));
    h.service.tokens().bind(&token("token-agent-4b"), next)?;

    // Then: the retired token is refused and the new program starts unreported.
    let retired = h.state(AgentState::Done, 2)?;
    assert_eq!(kind(&retired), Some("not_owner"));
    let status = h.status(AGENT)?.status;
    assert!(!status.reported);
    assert_eq!(status.process_incarnation, Some(Incarnation::new(2)));
    assert_eq!(status.turn, TurnNumber::new(1));
    let fresh = h.report("token-agent-4b", (StatusReport::new(AgentState::Idle), 1))?;
    assert_eq!(fresh.state, OpenEnum::Known(OpState::Committed));
    Ok(())
}

#[test]
fn turns_finish_only_when_the_root_leaves_working_and_reads_never_change_them() -> TestResult {
    let h = harness()?;
    let subscription = h.core.subscribe(None)?;
    h.state(AgentState::Working, 1)?;
    let mut child = StatusReport::new(AgentState::Done);
    child.id = Some("subagent".to_owned());
    h.report(AGENT_TOKEN, (child, 2))?;
    h.state(AgentState::Idle, 3)?;
    let first = h.status(AGENT)?;
    for _ in 0..3 {
        assert_eq!(h.status(AGENT)?, first);
        h.agent(AGENT)?;
    }
    assert_eq!(first.status.finish_generation, FinishGeneration::new(1));
    let turn = first.status.last_turn.clone().ok_or("finished turn")?;
    assert_eq!(turn.turn, TurnNumber::new(1));
    assert_eq!(turn.outcome, Some(OpenEnum::Known(TurnOutcome::Completed)));
    assert_eq!(turn.ended_on, Some(OpenEnum::Known(AgentState::Idle)));
    assert!(first.status.records.contains_key("subagent"));
    h.state(AgentState::Working, 4)?;
    h.state(AgentState::Error, 5)?;
    let second = h.status(AGENT)?.status;
    assert_eq!(second.turn, TurnNumber::new(2));
    assert_eq!(second.finish_generation, FinishGeneration::new(2));
    let failed = second.last_turn.ok_or("second turn")?;
    assert_eq!(failed.outcome, Some(OpenEnum::Known(TurnOutcome::Failed)));
    let ended = domain_events(&subscription)
        .into_iter()
        .filter(|event| event.event_type == "turn.ended")
        .count();
    assert_eq!(ended, 2);
    Ok(())
}

#[test]
fn unknown_states_out_of_range_progress_and_bad_labels_are_refused() -> TestResult {
    let h = harness()?;
    let mut unknown = StatusReport::new(AgentState::Idle);
    unknown.state = OpenEnum::Unknown("thinking".to_owned());
    assert_eq!(
        kind(&h.report(AGENT_TOKEN, (unknown, 1))?),
        Some("invalid_record")
    );
    let mut progress = StatusReport::new(AgentState::Working);
    progress.progress = Some(101);
    assert_eq!(
        kind(&h.report(AGENT_TOKEN, (progress, 1))?),
        Some("invalid_record")
    );
    assert_eq!(kind(&register(&h, AGENT_TOKEN, "")?), Some("invalid_agent"));
    assert!(!h.status(AGENT)?.status.reported);
    register(&h, AGENT_TOKEN, "first")?;
    assert_eq!(
        kind(&register(&h, AGENT_TOKEN, "second")?),
        Some("agent_conflict")
    );
    assert_eq!(
        kind(&register(&h, SHELL_TOKEN, "intruder")?),
        Some("not_owner")
    );
    Ok(())
}

#[test]
fn a_clear_ends_no_turn_and_an_exit_ends_a_working_turn() -> TestResult {
    let h = harness()?;
    h.state(AgentState::Working, 1)?;
    let clear = crate::status::Clear::new(terminal(AGENT)?, token(AGENT_TOKEN), None);
    h.run(|snapshot| h.service.clear(snapshot, &clear))?;
    let cleared = h.status(AGENT)?.status;
    assert_eq!(cleared.root, None);
    assert_eq!(cleared.finish_generation, FinishGeneration::new(0));
    h.state(AgentState::Working, 2)?;
    let owner = ProcessOwner::new(terminal(AGENT)?, Incarnation::new(1));
    let exited = h.service.exited(&h.core, owner)?;
    assert_eq!(exited.root, None);
    assert_eq!(exited.finish_generation, FinishGeneration::new(1));
    let turn = exited.last_turn.clone().ok_or("exited turn")?;
    assert_eq!(turn.outcome, Some(OpenEnum::Known(TurnOutcome::Exited)));
    assert_eq!(h.status(AGENT)?.status, exited);
    Ok(())
}

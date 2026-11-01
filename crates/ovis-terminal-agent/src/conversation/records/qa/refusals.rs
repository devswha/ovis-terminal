//! Refused appends change nothing; reads without structured records say so with
//! provenance instead of returning anything resembling chat.

use ovis_terminal_protocol::domain::conversation_records::{
    ConversationSource, MessagePhase, TurnState, UnavailableReason,
};
use ovis_terminal_protocol::errors::FieldPath;
use ovis_terminal_protocol::schema::ids::AgentId;
use ovis_terminal_protocol::schema::methods::agent::AgentResult;
use ovis_terminal_protocol::schema::scalar::OpenEnum;

use super::*;

fn refused_at(record: &OpRecord) -> Result<(String, Option<String>), Box<dyn Error>> {
    let error = refusal(record)?;
    Ok((
        error.kind.token().to_owned(),
        error
            .field_path
            .as_ref()
            .map(|path| FieldPath::as_str(path).to_owned()),
    ))
}

fn kind_at(kind: &str, path: &str) -> (String, Option<String>) {
    (kind.to_owned(), Some(path.to_owned()))
}

#[test]
fn a_dangling_tool_result_is_refused_and_appends_nothing() -> TestResult {
    // Given: a subscriber and a registered agent with nothing reported.
    let h = harness()?;
    let events = h.core.subscribe(None)?;
    h.register()?;

    // When: a valid record precedes a result that answers no call, in one append.
    let dangling = json!([
        text("u1", "user", "hi"),
        {"record_id": "u2", "role": "user", "blocks": [
            {"type": "text", "text": "result"},
            {"type": "tool_result", "call_id": "c-missing", "output": "x", "is_error": false}]}
    ]);
    let refused = h.append(&dangling)?;

    // Then: the whole append is refused at the call id, and the next event is a later append.
    assert_eq!(
        refused_at(&refused)?,
        kind_at("invalid_records", "params.records[1].blocks[1].call_id")
    );
    let unread = h.read_refusal(AGENT)?;
    assert_eq!(unread.kind.token(), "conversation_unavailable");
    h.append(&json!([text("u9", "user", "later")]))?;
    assert_eq!(ids(&h.page()?), ["u9"]);
    let event = next_appended(&events)?;
    let first = event.record_ids.first().ok_or("u9")?;
    assert_eq!(first.as_str(), "u9");
    Ok(())
}

#[test]
fn identities_roles_phases_and_turn_metadata_are_checked_against_the_contract() -> TestResult {
    let h = harness()?;
    h.register()?;
    let call = json!({"record_id": "a1", "role": "assistant", "blocks": [
        {"type": "tool_call", "call_id": "c1", "name": "read", "input": {}}]});
    h.append(&json!([call]))?;
    let cases = [
        (
            json!([text("a1", "user", "again")]),
            "params.records[0].record_id",
        ),
        (
            json!([{"record_id": "a2", "role": "assistant", "blocks": [
                {"type": "tool_call", "call_id": "c1", "name": "read", "input": {}}]}]),
            "params.records[0].blocks[0].call_id",
        ),
        (
            json!([{"record_id": "a3", "role": "assistant", "blocks": [
                {"type": "tool_result", "call_id": "c1", "output": "x", "is_error": false}]}]),
            "params.records[0].blocks[0].type",
        ),
        (
            json!([{"record_id": "u3", "role": "user", "phase": "final", "blocks": [
                {"type": "text", "text": "done?"}]}]),
            "params.records[0].phase",
        ),
        (
            json!([{"record_id": "u4", "role": "user", "blocks": []}]),
            "params.records[0].blocks",
        ),
        (json!([]), "params.records"),
    ];
    for (records, path) in cases {
        assert_eq!(
            refused_at(&h.append(&records)?)?,
            kind_at("invalid_records", path)
        );
    }
    let meta = serde_json::from_value(json!({"model": "m"}))?;
    let idle = h.append_as((TOKEN, &json!([])), Some(meta))?;
    assert_eq!(
        refused_at(&idle)?,
        kind_at("invalid_records", "params.turn")
    );
    assert_eq!(ids(&h.page()?), ["a1"]);
    Ok(())
}

#[test]
fn a_foreign_token_and_an_unregistered_program_are_not_owners() -> TestResult {
    let h = harness()?;
    let unregistered = h.append(&json!([text("u1", "user", "hi")]))?;
    h.register()?;
    let records = json!([text("u1", "user", "hi")]);
    let foreign = h.append_as(("token-shell-5", &records), None)?;

    assert_eq!(refusal(&unregistered)?.kind.token(), "not_owner");
    assert_eq!(refusal(&foreign)?.kind.token(), "not_owner");
    Ok(())
}

#[test]
fn a_plain_program_and_an_agent_without_records_are_unavailable_with_provenance() -> TestResult {
    // Given: a plain program on terminal 5 and a registered agent with nothing reported.
    let h = harness()?;
    let agent: AgentResult = result(&h.register()?)?;
    let agent_id: AgentId = agent.agent.ok_or("agent")?.agent_id;

    let plain = h.read_refusal(SHELL)?;
    let silent = h.read_refusal(AGENT)?;

    // Then: both are conversation_unavailable, naming the reason and what was consulted.
    assert_eq!(plain.kind.token(), "conversation_unavailable");
    let plain = plain.unavailable.ok_or("plain provenance")?;
    assert_eq!(plain.reason, OpenEnum::Known(UnavailableReason::NoAgent));
    assert_eq!(plain.provenance.agent_id, None);
    let silent = silent.unavailable.ok_or("agent provenance")?;
    assert_eq!(
        silent.reason,
        OpenEnum::Known(UnavailableReason::NotReported)
    );
    assert_eq!(
        silent.provenance.source,
        OpenEnum::Known(ConversationSource::Records)
    );
    assert_eq!(silent.provenance.agent_id, Some(agent_id));
    Ok(())
}

#[test]
fn commentary_in_an_open_turn_never_reads_as_a_finished_answer() -> TestResult {
    let h = harness()?;
    h.register()?;
    h.report(AgentState::Working, (1, 1_000))?;
    let commentary = json!([{"record_id": "a1", "role": "assistant", "phase": "commentary",
        "blocks": [{"type": "text", "text": "still looking"}]},
        text("a2", "assistant", "no phase recorded")]);
    h.append(&commentary)?;

    let page = h.page()?;

    let phases: Vec<_> = page.records.iter().map(|r| r.phase.clone()).collect();
    assert_eq!(
        phases,
        [Some(OpenEnum::Known(MessagePhase::Commentary)), None]
    );
    let turn = page.turns.first().ok_or("turn")?;
    assert_eq!(turn.state, OpenEnum::Known(TurnState::Open));
    assert_eq!(
        (turn.outcome.clone(), turn.ended_seq, turn.ended_ms),
        (None, None, None)
    );
    Ok(())
}

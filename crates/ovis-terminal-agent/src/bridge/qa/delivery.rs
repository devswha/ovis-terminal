//! Typed extension events: once per answer, in answer order, within the queue bound, and
//! acknowledged only at the version the answer committed.

use ovis_terminal_protocol::domain::prompt_bridge::DeliveryState;
use ovis_terminal_protocol::schema::methods::status::{PromptKind, PromptMechanism};
use ovis_terminal_protocol::schema::scalar::OpenEnum;

use super::{
    AGENT, AGENT_TOKEN, SHELL, SHELL_TOKEN, TestResult, fields, form, harness, ids, kind,
    prompt_id, token,
};
use crate::bridge::BridgeError;

#[test]
fn an_extension_answer_is_one_typed_event_acknowledged_at_its_version() -> TestResult {
    let h = harness(4)?;
    h.open(
        (AGENT, AGENT_TOKEN),
        ("f1", form(PromptMechanism::Extension)),
    )?;
    let (op, answered) = h.answer("f1", (1, fields(&[("branch", "main")])))?;
    assert_eq!(kind(&answered), None);
    let session = h.bridge.attach(&token(AGENT_TOKEN))?.token;
    let events = h.next(&session)?;
    assert_eq!(ids(&events), ["f1"]);
    let event = events.first().ok_or("no event")?;
    assert_eq!(event.version.get(), 2);
    assert_eq!(event.answer, fields(&[("branch", "main")]));
    assert_eq!(event.answer_op_id, op);
    assert_eq!(event.kind, OpenEnum::Known(PromptKind::Form));
    assert_eq!(ids(&h.next(&session)?), Vec::<&str>::new());
    let stale = h.ack(&session, ("f1", 1))?;
    assert!(matches!(stale, Err(BridgeError::VersionConflict { current }) if current.get() == 2));
    let record = h.ack(&session, ("f1", 2))?.map_err(|error| error.kind())?;
    assert_eq!(record.state, OpenEnum::Known(DeliveryState::Acknowledged));
    assert_eq!(record.answer_op_id, op);
    let again = h.ack(&session, ("f1", 2))?;
    assert_eq!(again.err().map(|error| error.kind()), Some("not_delivered"));
    let read = h.read(|snapshot| h.bridge.delivery(snapshot, &prompt_id("f1")))??;
    assert_eq!(read.state, OpenEnum::Known(DeliveryState::Acknowledged));
    Ok(())
}

#[test]
fn only_the_programs_own_extension_answers_reach_its_session() -> TestResult {
    let h = harness(4)?;
    h.open((AGENT, AGENT_TOKEN), ("r1", form(PromptMechanism::Rpc)))?;
    h.answer("r1", (1, fields(&[("branch", "main")])))?;
    h.open(
        (SHELL, SHELL_TOKEN),
        ("s1", form(PromptMechanism::Extension)),
    )?;
    h.answer("s1", (1, fields(&[("branch", "dev")])))?;
    h.open(
        (AGENT, AGENT_TOKEN),
        ("a1", form(PromptMechanism::Extension)),
    )?;
    let (_, outside) = h.answer("a1", (1, fields(&[("undeclared", "x")])))?;
    assert_eq!(kind(&outside), Some("answer_invalid"));
    let agent = h.bridge.attach(&token(AGENT_TOKEN))?.token;
    let shell = h.bridge.attach(&token(SHELL_TOKEN))?.token;
    assert_eq!(ids(&h.next(&agent)?), Vec::<&str>::new());
    assert_eq!(ids(&h.next(&shell)?), ["s1"]);
    let a1 = h.read(|snapshot| h.prompts.get(snapshot, &prompt_id("a1")))??;
    assert!(a1.is_open());
    let by_rpc = h.read(|snapshot| h.bridge.delivery(snapshot, &prompt_id("r1")))?;
    assert_eq!(by_rpc.err().map(|error| error.kind()), Some("not_found"));
    Ok(())
}

#[test]
fn the_queue_holds_at_most_its_limit_in_answer_order() -> TestResult {
    let h = harness(2)?;
    for id in ["e1", "e2", "e3"] {
        h.open((AGENT, AGENT_TOKEN), (id, form(PromptMechanism::Extension)))?;
    }
    for id in ["e3", "e1", "e2"] {
        h.answer(id, (1, fields(&[("branch", id)])))?;
    }
    let session = h.bridge.attach(&token(AGENT_TOKEN))?.token;
    assert_eq!(ids(&h.next(&session)?), ["e3", "e1"]);
    let full = h
        .try_next(&session)?
        .err()
        .ok_or("the full queue took more")?;
    assert_eq!(full.kind(), "limit_exceeded");
    let bound = full.error_data().bound.ok_or("no bound")?;
    let bound = serde_json::to_value(&bound)?;
    assert_eq!(bound.get("value"), Some(&serde_json::json!("2")));
    h.ack(&session, ("e3", 2))?.map_err(|error| error.kind())?;
    assert_eq!(ids(&h.next(&session)?), ["e2"]);
    let pending = h.read(|snapshot| h.bridge.delivery(snapshot, &prompt_id("e2")))??;
    assert_eq!(pending.state, OpenEnum::Known(DeliveryState::Delivered));
    Ok(())
}

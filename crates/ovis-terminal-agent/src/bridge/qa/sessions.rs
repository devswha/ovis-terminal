//! Sessions are opened only with a current process token, end with their program, and
//! never receive an answer a previous session already took.

use ovis_terminal_protocol::domain::prompt_bridge::DeliveryState;
use ovis_terminal_protocol::schema::methods::status::PromptMechanism;
use ovis_terminal_protocol::schema::scalar::{Incarnation, OpenEnum};

use super::{
    AGENT, AGENT_TOKEN, SHELL, SHELL_TOKEN, TestResult, fields, form, harness, ids, prompt_id,
    terminal, token,
};
use crate::status::ProcessOwner;

#[test]
fn a_session_needs_the_token_of_a_current_program() -> TestResult {
    let h = harness(2)?;
    let forged = h.bridge.attach(&token("other-process"));
    assert_eq!(forged.err().map(|error| error.kind()), Some("forbidden"));
    let shell = h.bridge.attach(&token(SHELL_TOKEN))?.attachment;
    assert_eq!(shell.terminal_id.as_str(), terminal(SHELL)?.to_string());
    assert_eq!(
        (shell.process_incarnation.get(), shell.queue_limit.get()),
        (1, 2)
    );
    let session = h.bridge.attach(&token(AGENT_TOKEN))?.token;
    let replaced = ProcessOwner::new(terminal(AGENT)?, Incarnation::new(2));
    h.status
        .tokens()
        .bind(&token("token-agent-4-next"), replaced)?;
    let ended = h.try_next(&session)?;
    assert_eq!(ended.err().map(|error| error.kind()), Some("forbidden"));
    let retired = h.bridge.attach(&token(AGENT_TOKEN));
    assert_eq!(retired.err().map(|error| error.kind()), Some("forbidden"));
    let successor = h.bridge.attach(&token("token-agent-4-next"))?.attachment;
    assert_eq!(successor.process_incarnation.get(), 2);
    Ok(())
}

#[test]
fn a_session_dropped_before_its_ack_is_never_redelivered() -> TestResult {
    let h = harness(2)?;
    h.open(
        (AGENT, AGENT_TOKEN),
        ("f1", form(PromptMechanism::Extension)),
    )?;
    let (op, _) = h.answer("f1", (1, fields(&[("branch", "main")])))?;
    let state = |id: &str| -> Result<_, Box<dyn std::error::Error>> {
        let record = h.read(|snapshot| h.bridge.delivery(snapshot, &prompt_id(id)))??;
        Ok(record.state)
    };
    assert_eq!(state("f1")?, OpenEnum::Known(DeliveryState::Pending));
    let first = h.bridge.attach(&token(AGENT_TOKEN))?.token;
    assert_eq!(ids(&h.next(&first)?), ["f1"]);
    assert_eq!(state("f1")?, OpenEnum::Known(DeliveryState::Delivered));
    h.bridge.detach(&first)?;
    assert_eq!(state("f1")?, OpenEnum::Known(DeliveryState::OutcomeUnknown));
    let late = h.ack(&first, ("f1", 2))?;
    assert_eq!(late.err().map(|error| error.kind()), Some("forbidden"));
    let second = h.bridge.attach(&token(AGENT_TOKEN))?.token;
    assert_eq!(ids(&h.next(&second)?), Vec::<&str>::new());
    let committed = h.read(|snapshot| h.prompts.get(snapshot, &prompt_id("f1")))??;
    assert_eq!(committed.answer.map(|receipt| receipt.op_id), Some(op));
    h.open(
        (AGENT, AGENT_TOKEN),
        ("f2", form(PromptMechanism::Extension)),
    )?;
    h.answer("f2", (1, fields(&[("branch", "dev")])))?;
    assert_eq!(ids(&h.next(&second)?), ["f2"]);
    let third = h.bridge.attach(&token(AGENT_TOKEN))?.token;
    assert_eq!(state("f2")?, OpenEnum::Known(DeliveryState::OutcomeUnknown));
    assert_eq!(ids(&h.next(&third)?), Vec::<&str>::new());
    let superseded = h.try_next(&second)?;
    assert_eq!(
        superseded.err().map(|error| error.kind()),
        Some("forbidden")
    );
    Ok(())
}

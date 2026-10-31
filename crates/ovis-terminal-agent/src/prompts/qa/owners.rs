//! Process-token ownership, bounded definitions, closes and replaced programs.

use ovis_terminal_protocol::errors::FieldPath;
use ovis_terminal_protocol::schema::methods::status::{
    PromptChoice, PromptCloseOutcome, PromptField, PromptState,
};
use ovis_terminal_protocol::schema::scalar::OpenEnum;

use super::*;
use crate::prompts::Close;

fn close(h: &Harness, by: &str, id: &str) -> Result<OpRecord, Box<dyn Error>> {
    let close = Close::new(
        (terminal(AGENT)?, token(by)),
        prompt_id(id),
        PromptCloseOutcome::Dismissed,
    );
    h.run(|snapshot| h.prompts.close(snapshot, &close))
}

#[test]
fn only_the_owning_program_opens_republishes_or_closes_its_prompt() -> TestResult {
    let h = harness()?;
    let forged = h.open("forged-token", ("p1", question(PromptMechanism::Rpc)))?;
    let other_terminal = h.open(SHELL_TOKEN, ("p1", question(PromptMechanism::Rpc)))?;
    let opened = result(&h.open(AGENT_TOKEN, ("p1", question(PromptMechanism::Rpc)))?)?;
    let mut shell_open = Open::new(
        terminal(SHELL)?,
        token(SHELL_TOKEN),
        question(PromptMechanism::Rpc),
    );
    shell_open.prompt_id = Some(prompt_id("p1"));
    let hijack = h.run(|snapshot| h.prompts.open(snapshot, shell_open))?;
    let foreign_close = close(&h, SHELL_TOKEN, "p1")?;
    let closed = result(&close(&h, AGENT_TOKEN, "p1")?)?;
    let twice = close(&h, AGENT_TOKEN, "p1")?;
    let reopen = h.open(AGENT_TOKEN, ("p1", question(PromptMechanism::Rpc)))?;

    assert_eq!(kind(&forged), Some("not_owner"));
    assert_eq!(kind(&other_terminal), Some("not_owner"));
    assert_eq!(kind(&hijack), Some("not_owner"));
    assert_eq!(kind(&foreign_close), Some("not_owner"));
    assert_eq!(opened.process_incarnation, Incarnation::new(1));
    assert_eq!(opened.terminal_id.as_str(), terminal(AGENT)?.to_string());
    assert_eq!(closed.state, OpenEnum::Known(PromptState::Dismissed));
    assert_eq!(closed.version, RecordVersion::new(2));
    assert_eq!(kind(&twice), Some("already_closed"));
    assert_eq!(kind(&reopen), Some("invalid_prompt"));
    Ok(())
}

#[test]
fn definitions_outside_their_bounds_are_refused_with_the_field() -> TestResult {
    let h = harness()?;
    let mut both = question(PromptMechanism::Rpc);
    both.choices
        .push(PromptChoice::new("a".to_owned(), "A".to_owned()));
    both.fields.push(PromptField::new("name".to_owned(), true));
    let mut duplicate = question(PromptMechanism::Rpc);
    duplicate.choices = vec![PromptChoice::new("a".to_owned(), "A".to_owned()); 2];
    let mut typed_form = question(PromptMechanism::Keys);
    typed_form
        .fields
        .push(PromptField::new("name".to_owned(), true));
    let mut empty = question(PromptMechanism::Rpc);
    empty.text.clear();
    for (definition, field) in [
        (both, "params.prompt.fields"),
        (duplicate, "params.prompt.choices"),
        (typed_form, "params.prompt.fields"),
        (empty, "params.prompt.text"),
    ] {
        let refused = h.open(AGENT_TOKEN, ("p1", definition))?;
        let error = refused.error.ok_or("refusal")?;
        assert_eq!(error.kind.token(), "invalid_prompt");
        assert_eq!(
            error.field_path.as_ref().map(FieldPath::as_str),
            Some(field)
        );
    }
    Ok(())
}

#[test]
fn a_replaced_programs_prompt_expires_on_answer_and_no_longer_blocks() -> TestResult {
    let h = harness()?;
    h.open(AGENT_TOKEN, ("p1", question(PromptMechanism::Rpc)))?;
    let subscription = h.core.subscribe(None)?;
    h.status.tokens().bind(
        &token("token-agent-4-second"),
        ProcessOwner::new(terminal(AGENT)?, Incarnation::new(2)),
    )?;

    let late = h.answer("p1", (1, text("yes")))?;

    assert_eq!(kind(&late), Some("prompt_closed"));
    let expired = h.get("p1")?;
    assert_eq!(expired.state, OpenEnum::Known(PromptState::Expired));
    assert_eq!(expired.answer, None);
    let events = domain_events(&subscription);
    assert_eq!(events.len(), 1);
    let event = events.first().ok_or("closed event")?;
    assert_eq!(event.event_type, "prompt.closed");
    assert_eq!(expired.closed_seq, Some(event.cursor.seq));
    assert!(h.admission(AGENT)?.is_ok());
    Ok(())
}

#[test]
fn a_restarted_service_routes_answers_to_retained_prompts() -> TestResult {
    let h = harness()?;
    h.open(AGENT_TOKEN, ("p1", question(PromptMechanism::Rpc)))?;
    let restarted = PromptService::new(h.identity.runtime_id(), Arc::clone(&h.status));
    assert_eq!(
        restarted
            .route(&prompt_id("p1"))
            .map_err(|error| error.kind()),
        Err("not_found")
    );
    h.read(|snapshot| restarted.recover(snapshot))??;
    assert_eq!(restarted.route(&prompt_id("p1"))?, terminal(AGENT)?);
    Ok(())
}

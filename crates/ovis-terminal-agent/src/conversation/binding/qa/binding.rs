//! Explicit, owner-only binding changes and their events.

use ovis_terminal_protocol::domain::session_binding::{HistoryChange, SessionChange};
use ovis_terminal_protocol::errors::FieldPath;
use ovis_terminal_protocol::schema::methods::server::OpState;
use ovis_terminal_protocol::schema::scalar::{HistoryRevision, OpenEnum};

use super::*;

#[test]
fn a_first_binding_and_a_resume_each_commit_one_explicit_event() -> TestResult {
    // Given: a registered agent and two transcripts in the authorized store.
    let h = harness()?;
    h.register()?;
    let first = h.write("one.jsonl", &[line("r1", None)])?;
    let second = h.write("two.jsonl", &[line("s1", None)])?;
    let subscription = h.core.subscribe(None)?;

    // When: the owner binds h1, then resumes h2.
    let bound: AgentResult = result(&h.bind((&first, "h1"), None)?)?;
    let resumed: AgentResult = result(&h.bind((&second, "h2"), Some(SessionChange::Resume))?)?;

    // Then: each change is one agent.session_bound event naming reason and predecessor.
    let events = domain_events(&subscription);
    let records: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.data {
            AgentStatusEvent::SessionBound(record) => Some((event.cursor.seq, record)),
            _ => None,
        })
        .collect();
    let [(first_seq, first_event), (second_seq, second_event)] = records.as_slice() else {
        return Err(format!("expected two binding events, got {events:?}").into());
    };
    assert_eq!(first_event.reason, None);
    assert_eq!(first_event.previous_history, None);
    assert_eq!(first_event.history.history_id, history("h1"));
    assert_eq!(first_event.history.revision, HistoryRevision::new(1));
    assert_eq!(first_event.binding_revision.get(), first_seq.get());
    assert_eq!(
        second_event.reason,
        Some(OpenEnum::Known(SessionChange::Resume))
    );
    assert_eq!(second_event.previous_history, Some(history("h1")));
    assert_eq!(
        second_event.previous_session.as_ref(),
        Some(&session(&first)?)
    );
    assert_eq!(second_event.binding_revision.get(), second_seq.get());
    let agent = resumed.agent.ok_or("agent")?;
    assert_eq!(agent.history_id, Some(history("h2")));
    assert!(agent.revision > bound.revision);
    assert_eq!(h.agent()?.agent, Some(agent));
    Ok(())
}

#[test]
fn once_bound_a_change_without_a_reason_is_refused_and_changes_nothing() -> TestResult {
    let h = harness()?;
    h.register()?;
    let first = h.write("one.jsonl", &[line("r1", None)])?;
    let second = h.write("two.jsonl", &[line("s1", None)])?;
    let bound: AgentResult = result(&h.bind((&first, "h1"), None)?)?;

    let refused = h.bind((&second, "h2"), None)?;

    assert_eq!(refused.state, OpenEnum::Known(OpState::Failed));
    let error = refusal(&refused)?;
    assert_eq!(error.kind.token(), "invalid_session");
    assert_eq!(
        error.field_path.as_ref().map(FieldPath::as_str),
        Some("params.reason")
    );
    assert_eq!(h.agent()?.agent, bound.agent);
    Ok(())
}

#[test]
fn a_replaced_programs_token_and_an_unregistered_successor_are_not_owners() -> TestResult {
    // Given: incarnation 1 registered and bound, then replaced by incarnation 2.
    let h = harness()?;
    h.register()?;
    let path = h.write("one.jsonl", &[line("r1", None)])?;
    result::<AgentResult>(&h.bind((&path, "h1"), None)?)?;
    h.status.tokens().bind(
        &token("token-agent-4-second"),
        ProcessOwner::new(terminal()?, Incarnation::new(2)),
    )?;

    // When: the old token rebinds, and the new program binds before registering.
    let stale = h.bind((&path, "h1"), Some(SessionChange::Resume))?;
    let mut fresh = Update::new(terminal()?, token("token-agent-4-second"), session(&path)?);
    fresh.history_id = Some(history("h1"));
    let unregistered = h.update(fresh)?;

    // Then: both are typed not_owner refusals.
    assert_eq!(refusal(&stale)?.kind.token(), "not_owner");
    let error = refusal(&unregistered)?;
    assert_eq!(error.kind.token(), "not_owner");
    assert!(error.hint.contains("agent.register"));
    Ok(())
}

#[test]
fn sources_outside_the_stores_and_rebinding_an_identity_elsewhere_are_invalid() -> TestResult {
    let h = harness()?;
    h.register()?;
    let inside = h.write("one.jsonl", &[line("r1", None)])?;
    let other = h.write("two.jsonl", &[line("s1", None)])?;
    let outside = h.root.join("outside.jsonl");
    std::fs::write(&outside, line("x1", None))?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, h.path("link.jsonl"))?;

    let escaped = h.bind((&outside, "h1"), None)?;
    assert_eq!(refusal(&escaped)?.kind.token(), "invalid_session");
    #[cfg(unix)]
    {
        let linked = h.bind((&h.path("link.jsonl"), "h1"), None)?;
        assert_eq!(refusal(&linked)?.kind.token(), "invalid_session");
    }
    let climbing = h.bind((Path::new("../outside.jsonl"), "h1"), None)?;
    assert_eq!(refusal(&climbing)?.kind.token(), "invalid_session");
    let mut unknown = Update::new(terminal()?, token(TOKEN), session(&inside)?);
    unknown.session.source = "unlisted-store".to_owned();
    let error = refusal(&h.update(unknown)?)?.clone();
    assert_eq!(
        error.field_path.as_ref().map(FieldPath::as_str),
        Some("params.session.source")
    );

    let relative: AgentResult = result(&h.bind((Path::new("one.jsonl"), "h1"), None)?)?;
    assert_eq!(
        relative.agent.ok_or("agent")?.history_id,
        Some(history("h1"))
    );
    let moved = h.bind((&other, "h1"), Some(SessionChange::Replace))?;
    let error = refusal(&moved)?;
    assert_eq!(error.kind.token(), "invalid_session");
    assert_eq!(
        error.field_path.as_ref().map(FieldPath::as_str),
        Some("params.history_id")
    );
    Ok(())
}

#[test]
fn a_branch_needs_a_known_head_and_a_head_needs_reason_branch() -> TestResult {
    let h = harness()?;
    h.register()?;
    let path = h.write("one.jsonl", &[line("r1", None), line("r2", None)])?;
    result::<AgentResult>(&h.bind((&path, "h1"), None)?)?;
    let field = |record: &OpRecord| {
        refusal(record).map(|error| error.field_path.as_ref().map(|p| p.as_str().to_owned()))
    };

    let headless = h.bind((&path, "h1"), Some(SessionChange::Branch))?;
    assert_eq!(field(&headless)?.as_deref(), Some("params.head"));
    let mut unknown = Update::new(terminal()?, token(TOKEN), session(&path)?);
    unknown.reason = Some(SessionChange::Branch);
    unknown.head = Some(record("r9"));
    assert_eq!(field(&h.update(unknown)?)?.as_deref(), Some("params.head"));
    let mut misplaced = Update::new(terminal()?, token(TOKEN), session(&path)?);
    misplaced.reason = Some(SessionChange::Resume);
    misplaced.head = Some(record("r1"));
    assert_eq!(
        field(&h.update(misplaced)?)?.as_deref(),
        Some("params.head")
    );

    let mut branch = Update::new(terminal()?, token(TOKEN), session(&path)?);
    branch.reason = Some(SessionChange::Branch);
    branch.head = Some(record("r1"));
    result::<AgentResult>(&h.update(branch)?)?;
    let page = h.page(None)?;
    assert_eq!(ids(&page), ["r1"]);
    assert_eq!(page.revision, HistoryRevision::new(2));
    assert_eq!(page.off_branch, [record("r2")]);
    let stale = h.read(Some(ConversationCursor::new(
        history("h1"),
        HistoryRevision::new(1),
        ovis_terminal_protocol::schema::scalar::RecordPosition::new(0),
    )))?;
    let current = refusal(&stale)?.history.clone().ok_or("history")?;
    assert_eq!(current.change, Some(OpenEnum::Known(HistoryChange::Branch)));
    Ok(())
}

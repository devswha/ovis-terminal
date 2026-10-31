//! Cursors bound to history identity and revision.

use std::io::Write as _;

use ovis_terminal_protocol::domain::session_binding::{HistoryChange, SessionChange};
use ovis_terminal_protocol::schema::common::PageRequest;
use ovis_terminal_protocol::schema::scalar::{HistoryRevision, OpenEnum, RecordPosition};

use super::*;

/// One provider write of the transcript under test.
type Writer<'a> = Box<dyn Fn() -> std::io::Result<()> + 'a>;

fn cursor(id: &str, revision: u64, position: u64) -> ConversationCursor {
    ConversationCursor::new(
        history(id),
        HistoryRevision::new(revision),
        RecordPosition::new(position),
    )
}

fn bound(lines: &[String]) -> Result<(Harness, PathBuf), Box<dyn Error>> {
    let h = harness()?;
    h.register()?;
    let path = h.write("one.jsonl", lines)?;
    result::<AgentResult>(&h.bind((&path, "h1"), None)?)?;
    Ok((h, path))
}

#[test]
fn an_append_refuses_the_old_cursor_with_the_current_revision_and_records_it() -> TestResult {
    // Given: h1 at revision 1 with two records read.
    let (h, path) = bound(&[line("r1", None), line("r2", None)])?;
    let first = h.page(None)?;
    assert_eq!(first.revision, HistoryRevision::new(1));
    assert_eq!(ids(&first), ["r1", "r2"]);

    // When: the provider appends r3 and the client continues from position 2 of revision 1.
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)?
        .write_all(line("r3", None).as_bytes())?;
    let stale = h.read(Some(cursor("h1", 1, 2)))?;

    // Then: history_changed names h1 at revision 2 and the append; no records are mixed.
    assert_eq!(stale.result, None);
    let error = refusal(&stale)?;
    assert_eq!(error.kind.token(), "history_changed");
    let current = error.history.clone().ok_or("history")?;
    assert_eq!(current.history_id, history("h1"));
    assert_eq!(current.revision, HistoryRevision::new(2));
    assert_eq!(current.change, Some(OpenEnum::Known(HistoryChange::Append)));
    // The refusal recorded revision 2: the same position now continues with r3 only.
    let continued = h.page(Some(cursor("h1", 2, 2)))?;
    assert_eq!(ids(&continued), ["r3"]);
    assert_eq!(continued.revision, HistoryRevision::new(2));
    Ok(())
}

#[test]
fn truncation_replacement_and_a_same_size_rewrite_each_advance_the_revision() -> TestResult {
    let (h, path) = bound(&[line("r1", None), line("r2", None)])?;
    let changes: [(&str, Writer<'_>, HistoryChange); 3] = [
        (
            "same-size rewrite",
            Box::new(|| std::fs::write(&path, [line("q1", None), line("q2", None)].concat())),
            HistoryChange::Rewrite,
        ),
        (
            "truncation",
            Box::new(|| std::fs::write(&path, line("q1", None))),
            HistoryChange::Truncate,
        ),
        (
            "replacement",
            Box::new(|| {
                let staged = path.with_extension("staged");
                std::fs::write(&staged, line("q1", None))?;
                std::fs::rename(&staged, &path)
            }),
            HistoryChange::Replace,
        ),
    ];
    let mut revision = 1;
    for (name, change, expected) in changes {
        let before = std::fs::metadata(&path)?.len();
        change()?;
        if expected == HistoryChange::Rewrite {
            assert_eq!(
                std::fs::metadata(&path)?.len(),
                before,
                "{name} keeps the size"
            );
        }
        let refused = h.read(Some(cursor("h1", revision, 0)))?;
        let current = refusal(&refused)?.history.clone().ok_or(name)?;
        revision += 1;
        assert_eq!(current.revision, HistoryRevision::new(revision), "{name}");
        assert_eq!(current.change, Some(OpenEnum::Known(expected)), "{name}");
    }
    let page = h.page(Some(cursor("h1", revision, 0)))?;
    assert_eq!(ids(&page), ["q1"]);
    Ok(())
}

#[test]
fn unchanged_reads_keep_the_revision_and_both_continuation_forms_agree() -> TestResult {
    let lines: Vec<String> = (1..=5).map(|n| line(&format!("r{n}"), None)).collect();
    let (h, _) = bound(&lines)?;
    let mut read = Read::new(terminal()?);
    read.page = serde_json::from_value::<Option<PageRequest>>(json!({"after": null, "limit": 2}))?;
    let first: AgentConversationResult =
        result(&h.run(|snapshot| h.service.conversation(snapshot, &read))?)?;
    assert_eq!(ids(&first), ["r1", "r2"]);
    assert_eq!(first.page.total, 5);
    let next = first.next_cursor.clone().ok_or("next cursor")?;
    assert_eq!(next.position, RecordPosition::new(2));

    let mut by_cursor = read.clone();
    by_cursor.cursor = Some(next);
    let mut by_token = read.clone();
    by_token.page = serde_json::from_value(json!({"after": first.page.next, "limit": 2}))?;
    let a: AgentConversationResult =
        result(&h.run(|snapshot| h.service.conversation(snapshot, &by_cursor))?)?;
    let b: AgentConversationResult =
        result(&h.run(|snapshot| h.service.conversation(snapshot, &by_token))?)?;
    assert_eq!(ids(&a), ["r3", "r4"]);
    assert_eq!(a, b);
    assert_eq!(a.revision, HistoryRevision::new(1));

    let mut forged = read;
    forged.page = serde_json::from_value(json!({"after": "not-a-cursor", "limit": 2}))?;
    let refused = h.run(|snapshot| h.service.conversation(snapshot, &forged))?;
    assert_eq!(refusal(&refused)?.kind.token(), "cursor_expired");
    let beyond = h.read(Some(cursor("h1", 1, 6)))?;
    assert_eq!(refusal(&beyond)?.kind.token(), "cursor_expired");
    Ok(())
}

#[test]
fn a_cursor_of_the_previous_history_names_the_rebound_identity() -> TestResult {
    let (h, _) = bound(&[line("r1", None)])?;
    let second = h.write("two.jsonl", &[line("s1", None)])?;
    result::<AgentResult>(&h.bind((&second, "h2"), Some(SessionChange::Replace))?)?;

    let refused = h.read(Some(cursor("h1", 1, 0)))?;

    let current = refusal(&refused)?.history.clone().ok_or("history")?;
    assert_eq!(current.history_id, history("h2"));
    assert_eq!(current.revision, HistoryRevision::new(1));
    assert_eq!(current.change, Some(OpenEnum::Known(HistoryChange::Rebind)));
    assert_eq!(ids(&h.page(None)?), ["s1"]);
    Ok(())
}

#[test]
fn reads_without_a_transcript_binding_are_unavailable_not_synthesized() -> TestResult {
    let h = harness()?;
    let unregistered = h.read(None)?;
    assert_eq!(
        refusal(&unregistered)?.kind.token(),
        "conversation_unavailable"
    );
    h.register()?;
    let mut by_id = Update::new(
        terminal()?,
        token(TOKEN),
        serde_json::from_value(json!({"source": "provider", "kind": "id", "value": "abc"}))?,
    );
    by_id.history_id = Some(history("h1"));
    result::<AgentResult>(&h.update(by_id)?)?;
    let refused = h.read(None)?;
    let error = refusal(&refused)?;
    assert_eq!(error.kind.token(), "conversation_unavailable");
    assert!(error.hint.contains("kind path"));
    Ok(())
}

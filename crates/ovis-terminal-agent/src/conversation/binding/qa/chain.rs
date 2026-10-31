//! Fork, rewind and continuation as explicit chain selection.

use std::io::Write as _;

use ovis_terminal_protocol::domain::session_binding::SessionChange;
use ovis_terminal_protocol::schema::scalar::{HistoryRevision, RecordPosition};
use proptest::prelude::*;

use super::*;
use crate::conversation::binding::chain::History;

fn append(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .append(true)
        .open(path)?
        .write_all(text.as_bytes())
}

#[test]
fn a_fork_keeps_the_abandoned_record_addressable_but_off_the_active_chain() -> TestResult {
    // Given: r1 <- r2 <- r3, bound as h1.
    let h = harness()?;
    h.register()?;
    let path = h.write(
        "one.jsonl",
        &[line("r1", None), line("r2", None), line("r3", None)],
    )?;
    result::<AgentResult>(&h.bind((&path, "h1"), None)?)?;

    // When: the provider forks r4 from r2.
    append(&path, &line("r4", Some("r2")))?;
    let active = h.page(None)?;

    // Then: the active chain skips r3, which is listed and readable through a cursor head.
    assert_eq!(ids(&active), ["r1", "r2", "r4"]);
    assert!(active.active);
    assert_eq!(active.off_branch, [record("r3")]);
    let mut off = ConversationCursor::new(history("h1"), active.revision, RecordPosition::new(0));
    off.head = Some(record("r3"));
    let abandoned = h.page(Some(off))?;
    assert_eq!(ids(&abandoned), ["r1", "r2", "r3"]);
    assert!(!abandoned.active);
    assert_eq!(abandoned.off_branch, [record("r4")]);
    let parents: Vec<Option<&str>> = abandoned
        .records
        .iter()
        .map(|record| record.parent.as_ref().map(RecordId::as_str))
        .collect();
    assert_eq!(parents, [None, Some("r1"), Some("r2")]);
    Ok(())
}

#[test]
fn a_rewind_selection_is_followed_by_records_that_continue_it() -> TestResult {
    let h = harness()?;
    h.register()?;
    let path = h.write(
        "one.jsonl",
        &[line("r1", None), line("r2", None), line("r3", None)],
    )?;
    result::<AgentResult>(&h.bind((&path, "h1"), None)?)?;
    let mut rewind = Update::new(terminal()?, token(TOKEN), session(&path)?);
    rewind.reason = Some(SessionChange::Branch);
    rewind.head = Some(record("r1"));
    result::<AgentResult>(&h.update(rewind)?)?;
    assert_eq!(ids(&h.page(None)?), ["r1"]);

    append(&path, &line("r5", Some("r1")))?;
    let page = h.page(None)?;

    assert_eq!(ids(&page), ["r1", "r5"]);
    assert_eq!(page.revision, HistoryRevision::new(3));
    assert_eq!(page.off_branch, [record("r3")]);
    Ok(())
}

#[test]
fn an_unfinished_line_is_not_a_record_and_a_malformed_one_is_unavailable() -> TestResult {
    let h = harness()?;
    h.register()?;
    let path = h.write("one.jsonl", &[line("r1", None)])?;
    result::<AgentResult>(&h.bind((&path, "h1"), None)?)?;
    append(&path, "{\"id\":\"r2\",\"ro")?;
    assert_eq!(ids(&h.page(None)?), ["r1"]);
    append(&path, "le\":\"user\"}\nnot json\n")?;
    let refused = h.read(None)?;
    assert_eq!(refusal(&refused)?.kind.token(), "conversation_unavailable");
    Ok(())
}

proptest! {
    #[test]
    fn every_chain_follows_parents_from_a_root_and_excludes_off_branch_leaves(
        parents in proptest::collection::vec(proptest::option::of(0_usize..64), 1..40),
    ) {
        let mut text = String::new();
        for (index, parent) in parents.iter().enumerate() {
            let parent = parent.filter(|parent| *parent < index).map(|parent| format!("n{parent}"));
            text.push_str(&line(&format!("n{index}"), parent.as_deref()));
        }
        let history = History::parse(text.as_bytes()).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let head = history.active_head(None).ok_or_else(|| TestCaseError::fail("head"))?;
        prop_assert_eq!(Some(head), parents.len().checked_sub(1));
        let chain = history.chain(head);
        prop_assert_eq!(chain.first().copied(), Some(0));
        for (parent, child) in chain.iter().zip(chain.iter().skip(1)) {
            let child = history.records.get(*child).ok_or_else(|| TestCaseError::fail("child"))?;
            prop_assert_eq!(child.parent, Some(*parent));
        }
        for leaf in history.off_branch(&chain) {
            let index = history.find(&leaf).ok_or_else(|| TestCaseError::fail("leaf"))?;
            prop_assert!(!chain.contains(&index));
        }
    }
}

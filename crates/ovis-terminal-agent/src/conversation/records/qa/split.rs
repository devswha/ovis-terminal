//! Records read the same however the owner split them into appends, and pages of any
//! size reassemble them; a cursor from an older revision is refused as an append.

use ovis_terminal_protocol::domain::session_binding::HistoryChange;
use ovis_terminal_protocol::schema::common::PageRequest;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use proptest::prelude::*;

use super::*;

/// `conversation()` appended in batches ending at each of `cuts`, then read whole.
fn appended_in(cuts: &[usize]) -> Result<AgentConversationResult, Box<dyn Error>> {
    let h = harness()?;
    h.register()?;
    h.report(AgentState::Working, (1, 1_000))?;
    let all = conversation();
    let records = all.as_array().ok_or("records")?;
    let mut start = 0;
    for end in cuts.iter().copied().chain([records.len()]) {
        let batch = records.get(start..end).ok_or("cut")?;
        if !batch.is_empty() {
            h.append(&Value::Array(batch.to_vec()))?;
        }
        start = end;
    }
    h.page()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn any_split_into_appends_reads_back_the_same_records(
        mut cuts in proptest::collection::vec(0usize..=4, 0..4)
    ) {
        cuts.sort_unstable();
        let whole = appended_in(&[]).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let split = appended_in(&cuts).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(split.records, whole.records);
        prop_assert_eq!(split.turns, whole.turns);
    }
}

#[test]
fn pages_of_two_reassemble_the_log_and_an_older_cursor_is_refused_as_an_append() -> TestResult {
    let h = harness()?;
    h.register()?;
    h.append(&conversation())?;
    let mut read = Read::new(terminal(AGENT)?);
    read.page = Some(serde_json::from_value::<PageRequest>(
        json!({"after": null, "limit": 2}),
    )?);
    let first: AgentConversationResult = result(&h.run(|s| h.records.conversation(s, &read))?)?;
    read.cursor.clone_from(&first.next_cursor);
    let second: AgentConversationResult = result(&h.run(|s| h.records.conversation(s, &read))?)?;

    assert_eq!(ids(&first), ["u1", "a1"]);
    assert_eq!(ids(&second), ["u2", "a2"]);
    assert_eq!(second.next_cursor, None);

    h.append(&json!([text("u3", "user", "more")]))?;
    let stale = refusal(&h.run(|s| h.records.conversation(s, &read))?)?.clone();
    assert_eq!(stale.kind.token(), "history_changed");
    let current = stale.history.ok_or("history")?;
    assert_eq!(current.revision.get(), 2);
    assert_eq!(current.change, Some(OpenEnum::Known(HistoryChange::Append)));
    Ok(())
}

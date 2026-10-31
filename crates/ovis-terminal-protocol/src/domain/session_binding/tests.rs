use proptest::prelude::*;
use serde_json::json;

use super::{ConversationCursor, ConversationPage, HistoryChange, HistoryHead};
use crate::errors::{BoundaryError, ErrorData, ErrorKind};
use crate::schema::ids::{HistoryId, RecordId};
use crate::schema::methods::agent::{
    AgentConversationParams, AgentConversationResult, AgentUpdateSessionParams,
};
use crate::schema::scalar::{HistoryRevision, RecordPosition};

fn conversation(cursor: &serde_json::Value) -> serde_json::Value {
    json!({"target": {"name": "agent"}, "page": null, "cursor": cursor})
}

#[test]
fn a_numeric_cursor_revision_is_refused_at_params_cursor_revision() -> Result<(), String> {
    let valid = conversation(&json!({"history_id": "h1", "revision": "1", "position": "0"}));
    serde_json::from_value::<AgentConversationParams>(valid).map_err(|e| e.to_string())?;
    let numeric = conversation(&json!({"history_id": "h1", "revision": 1, "position": "0"}));
    let refused = serde_path_to_error::deserialize::<_, AgentConversationParams>(numeric)
        .err()
        .ok_or("a numeric revision decoded")?;
    assert_eq!(refused.path().to_string(), "cursor.revision");
    Ok(())
}

#[test]
fn an_unknown_cursor_field_and_an_unknown_reason_are_refused() {
    let extra =
        conversation(&json!({"history_id": "h1", "revision": "1", "position": "0", "offset": "0"}));
    assert!(serde_json::from_value::<AgentConversationParams>(extra).is_err());
    let params = |reason: &str| {
        json!({"op_id": "o", "target": {"name": "agent"}, "process_token": "t",
               "session": {"source": "s", "kind": "path", "value": "/x"},
               "history_id": "h2", "reason": reason})
    };
    assert!(serde_json::from_value::<AgentUpdateSessionParams>(params("resume")).is_ok());
    assert!(serde_json::from_value::<AgentUpdateSessionParams>(params("guess")).is_err());
}

#[test]
fn history_changed_carries_the_current_identity_and_exact_revision() -> Result<(), serde_json::Error>
{
    let head = HistoryHead::new(
        HistoryId::new("h2".to_owned()),
        HistoryRevision::new(u64::MAX),
        Some(HistoryChange::Rewrite),
    );
    let error = BoundaryError::domain(ErrorKind::from("history_changed".to_owned()), false)
        .with_history(head.clone());
    let wire = serde_json::to_value(error.data())?;
    assert_eq!(
        wire.get("history"),
        Some(&json!({"history_id": "h2", "revision": "18446744073709551615", "change": "rewrite"}))
    );
    let back: ErrorData = serde_json::from_value(wire)?;
    assert_eq!(back.history.as_deref(), Some(&head));
    Ok(())
}

#[test]
fn a_page_publishes_its_continuation_as_both_cursor_and_token() -> Result<(), serde_json::Error> {
    let next = ConversationCursor::new(
        HistoryId::new("h1".to_owned()),
        HistoryRevision::new(3),
        RecordPosition::new(2),
    );
    let page = ConversationPage {
        history_id: HistoryId::new("h1".to_owned()),
        revision: HistoryRevision::new(3),
        head: Some(RecordId::new("r4".to_owned())),
        active: true,
        off_branch: vec![RecordId::new("r3".to_owned())],
        records: Vec::new(),
        total: 3,
        next: Some(next.clone()),
    };
    let result = AgentConversationResult::try_from(page)?;
    let token = result
        .page
        .next
        .as_ref()
        .ok_or_else(|| serde::de::Error::custom("token"))?;
    assert_eq!(ConversationCursor::from_token(token)?, next);
    assert_eq!(result.page.revision.get(), 3);
    assert_eq!(result.next_cursor, Some(next));
    Ok(())
}

proptest! {
    #[test]
    fn cursors_round_trip_as_decimal_strings(
        history in "[a-z0-9:-]{1,24}", revision in any::<u64>(), position in any::<u64>(),
        head in proptest::option::of("[a-z0-9]{1,8}"),
    ) {
        let mut cursor = ConversationCursor::new(
            HistoryId::new(history), HistoryRevision::new(revision), RecordPosition::new(position));
        cursor.head = head.map(RecordId::new);
        let wire = serde_json::to_value(&cursor).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(wire.get("revision"), Some(&json!(revision.to_string())));
        prop_assert_eq!(wire.get("position"), Some(&json!(position.to_string())));
        let back: ConversationCursor = serde_json::from_value(wire).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(&back, &cursor);
        let token = cursor.token().map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(ConversationCursor::from_token(&token).map_err(|e| TestCaseError::fail(e.to_string()))?, cursor);
    }
}

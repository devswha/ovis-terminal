//! One append checked against every record before it, into the records reads return.

use std::collections::{BTreeMap, BTreeSet};

use ovis_terminal_protocol::domain::conversation_records::{
    ContentBlock, MessagePhase, RecordBlock, RecordInput, RecordRole, ToolLink,
};
use ovis_terminal_protocol::schema::ids::{CallId, RecordId};
use ovis_terminal_protocol::schema::methods::agent::ConversationRecord;
use ovis_terminal_protocol::schema::scalar::{OpenEnum, TurnNumber};

use super::log::{RECORD_LIMIT, RecordLog};
use super::{RecordsError, Violation};

/// Most records one append carries.
const BATCH_LIMIT: usize = 64;
/// Most blocks one record carries.
const BLOCK_LIMIT: usize = 32;
/// Largest identity or tool name, in bytes.
const NAME_LIMIT: usize = 256;
/// Largest text, tool input or tool output, in bytes.
const TEXT_LIMIT: usize = 64 * 1024;
/// Provenance of every reported record.
pub(super) const PROVENANCE: &str = "records";

/// Identities and calls already taken, and the record a new one continues.
struct Seen {
    records: BTreeSet<RecordId>,
    calls: BTreeMap<CallId, RecordId>,
    last: Option<RecordId>,
}

impl Seen {
    fn of(log: &RecordLog) -> Self {
        let mut calls = BTreeMap::new();
        for record in &log.records {
            for block in &record.blocks {
                if let RecordBlock::Typed(ContentBlock::ToolCall { call_id, .. }) = block {
                    calls.insert(call_id.clone(), record.record_id.clone());
                }
            }
        }
        Self {
            records: log.records.iter().map(|r| r.record_id.clone()).collect(),
            calls,
            last: log.records.last().map(|record| record.record_id.clone()),
        }
    }
}

const fn carries(role: RecordRole, block: &ContentBlock) -> bool {
    matches!(
        (role, block),
        (
            RecordRole::User,
            ContentBlock::Text { .. } | ContentBlock::ToolResult { .. }
        ) | (
            RecordRole::Assistant,
            ContentBlock::Text { .. } | ContentBlock::ToolCall { .. }
        ) | (RecordRole::Tool, ContentBlock::ToolResult { .. })
    )
}

const fn named(value: &str) -> bool {
    !value.is_empty() && value.len() <= NAME_LIMIT
}

/// Checks one block of the record at `at`, recording calls and the links of results.
fn block(
    seen: &mut Seen,
    (at, record): (&str, &RecordInput),
    (index, block): (usize, &ContentBlock),
) -> Result<Option<ToolLink>, RecordsError> {
    let field = |rest: &str| format!("{at}.blocks[{index}]{rest}");
    let bounds = |rest: &str| RecordsError::invalid(field(rest), Violation::Bounds);
    if !carries(record.role, block) {
        return Err(RecordsError::invalid(field(".type"), Violation::BlockRole));
    }
    match block {
        ContentBlock::Text { text } if text.len() > TEXT_LIMIT => Err(bounds(".text")),
        ContentBlock::ToolCall { call_id, .. } if !named(call_id.as_str()) => {
            Err(bounds(".call_id"))
        }
        ContentBlock::ToolCall { name, .. } if !named(name) => Err(bounds(".name")),
        ContentBlock::ToolCall { input, .. } if serde_json::to_vec(input)?.len() > TEXT_LIMIT => {
            Err(bounds(".input"))
        }
        ContentBlock::ToolCall { call_id, .. } => {
            if seen.calls.contains_key(call_id) {
                return Err(RecordsError::invalid(
                    field(".call_id"),
                    Violation::DuplicateCall,
                ));
            }
            seen.calls.insert(call_id.clone(), record.record_id.clone());
            Ok(None)
        }
        ContentBlock::ToolResult { output, .. } if output.len() > TEXT_LIMIT => {
            Err(bounds(".output"))
        }
        ContentBlock::ToolResult { call_id, .. } => {
            let call = seen
                .calls
                .get(call_id)
                .ok_or_else(|| RecordsError::invalid(field(".call_id"), Violation::DanglingCall))?;
            Ok(Some(ToolLink::new(call_id.clone(), call.clone())))
        }
        _ => Ok(None),
    }
}

fn record(
    seen: &mut Seen,
    (index, input): (usize, &RecordInput),
    turn: Option<TurnNumber>,
) -> Result<ConversationRecord, RecordsError> {
    let at = format!("records[{index}]");
    let invalid = |rest: &str, violation| RecordsError::invalid(format!("{at}{rest}"), violation);
    if !named(input.record_id.as_str()) {
        return Err(invalid(".record_id", Violation::Bounds));
    }
    if seen.records.contains(&input.record_id) {
        return Err(invalid(".record_id", Violation::DuplicateRecord));
    }
    if input.blocks.is_empty() {
        return Err(invalid(".blocks", Violation::Empty));
    }
    if input.blocks.len() > BLOCK_LIMIT {
        return Err(invalid(".blocks", Violation::Bounds));
    }
    if input.phase.is_some() && input.role != RecordRole::Assistant {
        return Err(invalid(".phase", Violation::PhaseRole));
    }
    let mut links = Vec::new();
    for indexed in input.blocks.iter().enumerate() {
        links.extend(block(seen, (&at, input), indexed)?);
    }
    let role = serde_json::from_value(serde_json::to_value(input.role)?)?;
    let blocks = input
        .blocks
        .iter()
        .cloned()
        .map(RecordBlock::Typed)
        .collect();
    let mut out =
        ConversationRecord::new(input.record_id.clone(), seen.last.clone(), (role, blocks));
    PROVENANCE.clone_into(&mut out.provenance);
    out.phase = input.phase.map(OpenEnum::<MessagePhase>::Known);
    out.turn = turn;
    out.at_ms = input.at_ms;
    out.links = links;
    seen.records.insert(input.record_id.clone());
    seen.last = Some(input.record_id.clone());
    Ok(out)
}

/// The records of `inputs` as reads will return them, stamped with the running `turn`.
pub(super) fn append(
    log: &RecordLog,
    inputs: &[RecordInput],
    turn: Option<TurnNumber>,
) -> Result<Vec<ConversationRecord>, RecordsError> {
    if inputs.len() > BATCH_LIMIT {
        return Err(RecordsError::invalid(
            "records".to_owned(),
            Violation::Bounds,
        ));
    }
    if log.records.len().saturating_add(inputs.len()) > RECORD_LIMIT {
        return Err(RecordsError::TooLarge);
    }
    let mut seen = Seen::of(log);
    inputs
        .iter()
        .enumerate()
        .map(|indexed| record(&mut seen, indexed, turn))
        .collect()
}

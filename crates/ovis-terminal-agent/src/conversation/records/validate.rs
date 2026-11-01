//! One append checked against every record before it, into the records reads return.

use std::collections::{BTreeMap, BTreeSet};

use ovis_terminal_protocol::domain::conversation_records::{
    ContentBlock, MessagePhase, RecordBlock, RecordInput, RecordRole, ToolLink,
};
use ovis_terminal_protocol::schema::ids::{CallId, RecordId};
use ovis_terminal_protocol::schema::methods::agent::ConversationRecord;
use ovis_terminal_protocol::schema::scalar::{OpenEnum, RecordPosition, TurnNumber};
use ovis_terminal_store::Snapshot;

use super::store::{Entry, Head, Keys};
use super::{RecordsError, Violation};

/// Most records one append carries.
pub(super) const BATCH_LIMIT: usize = 64;
/// Most blocks one record carries.
const BLOCK_LIMIT: usize = 32;
/// Largest record identity, in bytes; 64 of them keep an append's receipt small.
pub(super) const ID_LIMIT: usize = 128;
/// Largest call identity, tool name, model or reasoning label, in bytes.
pub(super) const NAME_LIMIT: usize = 256;
/// Largest text, tool input or tool output, in bytes.
pub(super) const TEXT_LIMIT: usize = 64 * 1024;
/// Largest stored record, in bytes; the default bound of one request's params.
pub(super) const RECORD_BYTES: usize = 256 * 1024;
/// Provenance of every reported record.
const PROVENANCE: &str = "records";

/// The stored log an append is checked against.
pub(super) struct Log<'a> {
    pub snapshot: &'a Snapshot,
    pub keys: &'a Keys,
    pub head: &'a Head,
}

/// One accepted record and its stored encoding.
pub(super) struct Accepted {
    pub record: ConversationRecord,
    pub bytes: Vec<u8>,
    /// Calls this record makes, indexed to it.
    pub calls: Vec<CallId>,
}

/// Identities this append already takes, and the record a new one continues.
struct Seen<'a> {
    log: &'a Log<'a>,
    records: BTreeSet<RecordId>,
    calls: BTreeMap<CallId, RecordId>,
    last: Option<RecordId>,
}

impl Seen<'_> {
    fn taken(&self, id: &RecordId) -> Result<bool, RecordsError> {
        if self.records.contains(id) {
            return Ok(true);
        }
        let stored = Entry::<RecordPosition>::read(self.log.snapshot, self.log.keys.id(id))?;
        Ok(stored.value.is_some())
    }

    fn call(&self, id: &CallId) -> Result<Option<RecordId>, RecordsError> {
        if let Some(record) = self.calls.get(id) {
            return Ok(Some(record.clone()));
        }
        Ok(Entry::<RecordId>::read(self.log.snapshot, self.log.keys.call(id))?.value)
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

const fn named(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit
}

/// Checks one block of the record at `at`, recording calls and the links of results.
fn block(
    seen: &mut Seen<'_>,
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
        ContentBlock::ToolCall { call_id, .. } if !named(call_id.as_str(), NAME_LIMIT) => {
            Err(bounds(".call_id"))
        }
        ContentBlock::ToolCall { name, .. } if !named(name, NAME_LIMIT) => Err(bounds(".name")),
        ContentBlock::ToolCall { input, .. } if serde_json::to_vec(input)?.len() > TEXT_LIMIT => {
            Err(bounds(".input"))
        }
        ContentBlock::ToolCall { call_id, .. } => {
            if seen.call(call_id)?.is_some() {
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
                .call(call_id)?
                .ok_or_else(|| RecordsError::invalid(field(".call_id"), Violation::DanglingCall))?;
            Ok(Some(ToolLink::new(call_id.clone(), call)))
        }
        _ => Ok(None),
    }
}

fn calls(input: &RecordInput) -> Vec<CallId> {
    input
        .blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolCall { call_id, .. } => Some(call_id.clone()),
            _ => None,
        })
        .collect()
}

fn record(
    seen: &mut Seen<'_>,
    (index, input): (usize, &RecordInput),
    turn: Option<TurnNumber>,
) -> Result<Accepted, RecordsError> {
    let at = format!("records[{index}]");
    let invalid = |rest: &str, violation| RecordsError::invalid(format!("{at}{rest}"), violation);
    if !named(input.record_id.as_str(), ID_LIMIT) {
        return Err(invalid(".record_id", Violation::Bounds));
    }
    if seen.taken(&input.record_id)? {
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
    let bytes = serde_json::to_vec(&out)?;
    if bytes.len() > RECORD_BYTES {
        return Err(invalid("", Violation::Bounds));
    }
    seen.records.insert(input.record_id.clone());
    seen.last = Some(input.record_id.clone());
    Ok(Accepted {
        record: out,
        bytes,
        calls: calls(input),
    })
}

/// The records of `inputs` as reads will return them, stamped with the running `turn`.
pub(super) fn append(
    log: &Log<'_>,
    inputs: &[RecordInput],
    turn: Option<TurnNumber>,
) -> Result<Vec<Accepted>, RecordsError> {
    if inputs.len() > BATCH_LIMIT {
        return Err(RecordsError::invalid(
            "records".to_owned(),
            Violation::Bounds,
        ));
    }
    let mut seen = Seen {
        log,
        records: BTreeSet::new(),
        calls: BTreeMap::new(),
        last: log.head.last.clone(),
    };
    inputs
        .iter()
        .enumerate()
        .map(|indexed| record(&mut seen, indexed, turn))
        .collect()
}

//! `agent.append_records`: typed records reported by the owning process, committed with
//! their log and the `agent.records_appended` event in one unit.

use std::sync::Arc;

use ovis_terminal_core::events::{EventError, ResourceId};
use ovis_terminal_core::ops::{Completion, OperationError};
use ovis_terminal_core::registry::{RuntimeId, TerminalId};
use ovis_terminal_protocol::domain::conversation_records::{
    AppendFacts, ConversationTurn, RecordInput, RecordsAppended, RecordsDigest, TurnMetadata,
};
use ovis_terminal_protocol::schema::ids::ProcessToken;
use ovis_terminal_protocol::schema::scalar::{
    ByteCount, HistoryRevision, RecordCount, RecordPosition, TurnNumber,
};
use ovis_terminal_store::{CasWrite, Snapshot, Version};
use sha2::{Digest as _, Sha256};

use super::store::{self, BYTE_LIMIT, Entry, Head, Keys, RECORD_LIMIT};
use super::turns::{TurnFacts, running};
use super::validate::{self, Accepted, Log, NAME_LIMIT};
use super::{RecordsError, Violation};
use crate::status::row::Row;
use crate::status::{AgentStatusEvent, StatusError, StatusService};

/// An `agent.append_records` by the process holding `token`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Append {
    /// Resolved target terminal.
    pub terminal: TerminalId,
    /// Token presented by the reporter.
    pub token: ProcessToken,
    /// Records to append, in order.
    pub records: Vec<RecordInput>,
    /// Metadata of the running turn.
    pub turn: Option<TurnMetadata>,
}

impl Append {
    /// Appends `records` for the program holding `token` on `terminal`.
    #[must_use]
    pub const fn new(terminal: TerminalId, token: ProcessToken, records: Vec<RecordInput>) -> Self {
        Self {
            terminal,
            token,
            records,
            turn: None,
        }
    }
}

/// Typed conversation records of every agent of one runtime.
#[derive(Debug)]
pub struct RecordsService {
    pub(super) runtime: RuntimeId,
    pub(super) status: Arc<StatusService>,
}

/// Settles a refusal as a failed operation; persistence failures stay errors.
pub(super) fn settle<D>(
    outcome: Result<Completion<D>, RecordsError>,
) -> Result<Completion<D>, OperationError> {
    match outcome {
        Ok(completion) => Ok(completion),
        Err(RecordsError::Encode(error) | RecordsError::Status(StatusError::Encode(error))) => {
            Err(error.into())
        }
        Err(RecordsError::Event(error) | RecordsError::Status(StatusError::Event(error))) => {
            Err(error.into())
        }
        Err(RecordsError::Status(StatusError::Poisoned)) => Err(EventError::Poisoned.into()),
        Err(refusal) => Ok(Completion::failed(refusal.error_data())),
    }
}

/// Checks reported turn metadata: bounded labels, and a running turn to carry them.
fn metadata(
    meta: Option<&TurnMetadata>,
    running: Option<TurnNumber>,
) -> Result<Option<TurnNumber>, RecordsError> {
    let Some(meta) = meta else {
        return Ok(None);
    };
    for (field, label) in [
        ("turn.model", &meta.model),
        ("turn.reasoning", &meta.reasoning),
    ] {
        if label
            .as_ref()
            .is_some_and(|label| label.is_empty() || label.len() > NAME_LIMIT)
        {
            return Err(RecordsError::invalid(field.to_owned(), Violation::Bounds));
        }
    }
    running
        .map(Some)
        .ok_or_else(|| RecordsError::invalid("turn".to_owned(), Violation::NoRunningTurn))
}

/// Folds status facts, and reported metadata, into the entries of the turns this append
/// touches and of the turn status last finished; returns their writes and the views of
/// the touched turns.
fn turns(
    (snapshot, keys, row): (&Snapshot, &Keys, &Row),
    (touched, meta): (Option<TurnNumber>, Option<TurnMetadata>),
) -> Result<(Vec<CasWrite>, Vec<ConversationTurn>), RecordsError> {
    let ended = row.status.last_turn.as_ref().map(|turn| turn.turn);
    let mut writes = Vec::new();
    let mut views = Vec::new();
    let mut numbers: Vec<TurnNumber> = touched.into_iter().chain(ended).collect();
    numbers.dedup();
    for turn in numbers {
        let entry = Entry::<TurnFacts>::read(snapshot, keys.turn(turn))?;
        let is_touched = Some(turn) == touched;
        let Some(mut facts) = entry
            .value
            .clone()
            .or_else(|| is_touched.then(TurnFacts::default))
        else {
            continue;
        };
        if let Some(meta) = meta.as_ref().filter(|_| is_touched) {
            if meta.model.is_some() {
                facts.model.clone_from(&meta.model);
            }
            if meta.reasoning.is_some() {
                facts.reasoning.clone_from(&meta.reasoning);
            }
        }
        facts.observe(turn, row);
        if entry.value.as_ref() != Some(&facts) {
            writes.push(entry.write(&facts)?);
        }
        if is_touched {
            views.push(facts.view(turn, row));
        }
    }
    Ok((writes, views))
}

/// `sha256:` and the hex digest of the appended records' stored encodings, in order.
fn digest(accepted: &[Accepted]) -> RecordsDigest {
    let mut hasher = Sha256::new();
    for record in accepted {
        hasher.update(&record.bytes);
    }
    RecordsDigest::new(format!("sha256:{:x}", hasher.finalize()))
}

/// The writes of the accepted records and of their identity and call index entries.
fn record_writes(
    keys: &Keys,
    head: &Head,
    accepted: &[Accepted],
) -> Result<Vec<CasWrite>, RecordsError> {
    let mut writes = Vec::new();
    for (position, record) in (head.count..).zip(accepted) {
        let key = keys.record(position);
        writes.push(store::put(key, Version::ABSENT, &record.bytes));
        let id = &record.record.record_id;
        writes.push(store::create(keys.id(id), &RecordPosition::new(position))?);
        for call in &record.calls {
            writes.push(store::create(keys.call(call), id)?);
        }
    }
    Ok(writes)
}

impl RecordsService {
    /// Records of the agents `status` registers.
    #[must_use]
    pub const fn new(runtime: RuntimeId, status: Arc<StatusService>) -> Self {
        Self { runtime, status }
    }

    /// Appends typed records, and metadata of the running turn, as the owning process.
    ///
    /// Each record is stored as its own entry; the result names them by identity, count
    /// and digest, so the operation receipt stays small whatever they hold.
    ///
    /// # Errors
    ///
    /// Refusals (`not_owner`, `invalid_records`, `too_large`) settle the operation as
    /// failed with nothing appended; a store or decoding failure is returned.
    pub fn append<D: From<AgentStatusEvent>>(
        &self,
        snapshot: &Snapshot,
        append: Append,
    ) -> Result<Completion<D>, OperationError> {
        settle((|| {
            let owner = self
                .status
                .tokens()
                .authorize(&append.token, append.terminal)?;
            let stored = self
                .status
                .location(owner.terminal)
                .read(snapshot, Some(owner.process))?;
            let agent = stored
                .row
                .agent
                .as_ref()
                .ok_or(RecordsError::Unregistered)?;
            if append.records.is_empty() && append.turn.is_none() {
                return Err(RecordsError::invalid(
                    "records".to_owned(),
                    Violation::Empty,
                ));
            }
            let keys = Keys::new(self.runtime, &agent.agent_id);
            let head_entry = Entry::<Head>::read(snapshot, keys.head())?;
            let mut head = head_entry.value.clone().unwrap_or_else(Head::empty);
            let running = running(&stored.row);
            let log = Log {
                snapshot,
                keys: &keys,
                head: &head,
            };
            let accepted = validate::append(&log, &append.records, running)?;
            let meta_turn = metadata(append.turn.as_ref(), running)?;
            let large = |_| RecordsError::TooLarge;
            let added = accepted
                .iter()
                .map(|r| u64::try_from(r.bytes.len()))
                .sum::<Result<u64, _>>()
                .map_err(large)?;
            let count = head
                .count
                .saturating_add(u64::try_from(accepted.len()).map_err(large)?);
            let bytes = head.bytes.saturating_add(added);
            if count > RECORD_LIMIT || bytes > BYTE_LIMIT {
                return Err(RecordsError::TooLarge);
            }
            let touched = meta_turn.or_else(|| running.filter(|_| !accepted.is_empty()));
            let (turn_writes, views) =
                turns((snapshot, &keys, &stored.row), (touched, append.turn))?;
            let writes = record_writes(&keys, &head, &accepted)?;
            let first = head.count;
            head.count = count;
            head.bytes = bytes;
            head.revision = HistoryRevision::new(head.revision.get().saturating_add(1));
            if let Some(last) = accepted.last() {
                head.last = Some(last.record.record_id.clone());
            }
            let appended = RecordsAppended::from(AppendFacts {
                agent_id: agent.agent_id.clone(),
                process_incarnation: owner.process,
                history_id: head.history_id.clone(),
                revision: head.revision,
                record_ids: accepted
                    .iter()
                    .map(|r| r.record.record_id.clone())
                    .collect(),
                first_position: RecordPosition::new(first),
                total_records: RecordCount::new(count),
                total_bytes: ByteCount::new(bytes),
                digest: digest(&accepted),
                turns: views,
            });
            let mut completion =
                Completion::new(serde_json::to_value(&appended)?).write(head_entry.write(&head)?);
            for write in writes.into_iter().chain(turn_writes) {
                completion = completion.write(write);
            }
            Ok(completion.event(
                D::from(AgentStatusEvent::RecordsAppended(Box::new(appended))),
                vec![ResourceId::Terminal(owner.terminal)],
            ))
        })())
    }
}

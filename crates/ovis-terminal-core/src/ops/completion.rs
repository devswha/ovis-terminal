//! Method effects and their operation outcome share one coordinator transaction.

use ovis_terminal_protocol::errors::ErrorData;
use ovis_terminal_protocol::schema::methods::server::{OpRecord, OpState, OperationEvidence};
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use ovis_terminal_store::{CasWrite, OpId, Payload, Snapshot, Version};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::context::encode_entry;
use super::ledger::Row;
use super::{OperationError, OperationEvent, OperationSnapshot, Operations, RunningOperation};
use crate::events::{Commit, EventData, ResourceId, SnapshotProducer};

/// Typed domain writes and events accompanying a method's JSON result.
#[derive(Debug)]
pub struct Completion<D> {
    outcome: Outcome,
    writes: Vec<CasWrite>,
    events: Vec<(D, Vec<ResourceId>)>,
    evidence: Option<OperationEvidence>,
}

#[derive(Debug)]
enum Outcome {
    Pending,
    Result(Value),
    Failed(ErrorData),
}

impl<D> Completion<D> {
    /// Creates a completion without domain writes.
    pub const fn new(result: Value) -> Self {
        Self {
            outcome: Outcome::Result(result),
            writes: Vec::new(),
            events: Vec::new(),
            evidence: None,
        }
    }

    /// Creates a running transition with no premature method result.
    pub const fn pending() -> Self {
        Self {
            outcome: Outcome::Pending,
            writes: Vec::new(),
            events: Vec::new(),
            evidence: None,
        }
    }

    /// Records a proven failure before an external effect, with its domain state.
    pub const fn failed(error: ErrorData) -> Self {
        Self {
            outcome: Outcome::Failed(error),
            writes: Vec::new(),
            events: Vec::new(),
            evidence: None,
        }
    }

    /// Adds a state write to the operation's transaction.
    #[must_use]
    pub fn write(mut self, write: CasWrite) -> Self {
        self.writes.push(write);
        self
    }

    /// Adds a domain event after the operation event in commit order.
    #[must_use]
    pub fn event(mut self, data: D, affected: Vec<ResourceId>) -> Self {
        self.events.push((data, affected));
        self
    }

    /// Binds factual process evidence to the same persisted outcome.
    #[must_use]
    pub fn evidence(mut self, evidence: OperationEvidence) -> Self {
        self.evidence = Some(evidence);
        self
    }
}

impl<D, P> Operations<'_, D, P>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned,
    P: SnapshotProducer,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    /// Commits the operation outcome and all domain effects in one SQLite transaction.
    ///
    /// # Errors
    ///
    /// Returns stale permission, construction, CAS or persistence failures.
    pub fn commit_unit(
        &self,
        permit: RunningOperation,
        build: impl FnOnce(&Snapshot) -> Result<Completion<D>, OperationError>,
    ) -> Result<OpRecord, OperationError> {
        let mut row = self.permitted(&permit)?;
        if let Some(evidence) = &row.entry.record.evidence
            && let Some(intent) = &evidence.intent
            && evidence
                .write
                .as_ref()
                .is_none_or(|write| write.written_bytes != intent.requested_bytes)
        {
            let unknown = self.outcome_unknown(permit)?;
            return Err(OperationError::Settled(Box::new(unknown)));
        }
        row.entry.record.state = OpenEnum::Known(OpState::Committed);
        self.persist_unit(row, build)
    }

    /// Persists a pending domain transition without declaring the running operation complete.
    ///
    /// # Errors
    ///
    /// Returns stale permission, construction, CAS or persistence failures.
    pub fn record_unit(
        &self,
        permit: &RunningOperation,
        build: impl FnOnce(&Snapshot) -> Result<Completion<D>, OperationError>,
    ) -> Result<OpRecord, OperationError> {
        self.persist_unit(self.permitted(permit)?, build)
    }

    fn persist_unit(
        &self,
        mut row: Row,
        build: impl FnOnce(&Snapshot) -> Result<Completion<D>, OperationError>,
    ) -> Result<OpRecord, OperationError> {
        let op_id = row.entry.record.op_id.clone();
        let cursor = self
            .coordinator
            .commit_unit(|snapshot| {
                let completion = build(snapshot)?;
                match (row.entry.record.state.clone(), completion.outcome) {
                    (OpenEnum::Known(OpState::Running), Outcome::Pending) => {}
                    (OpenEnum::Known(OpState::Committed), Outcome::Result(result)) => {
                        row.entry.record.result = Some(result);
                    }
                    (OpenEnum::Known(OpState::Committed), Outcome::Failed(error)) => {
                        row.entry.record.state = OpenEnum::Known(OpState::Failed);
                        row.entry.record.error = Some(error);
                    }
                    _ => return Err(OperationError::InvalidParams),
                }
                if let Some(mut evidence) = completion.evidence {
                    if let Some(previous) = &row.entry.record.evidence {
                        if previous
                            .terminal_id
                            .as_ref()
                            .is_some_and(|id| Some(id) != evidence.terminal_id.as_ref())
                            || previous.intent != evidence.intent
                            || previous.write != evidence.write
                            || previous
                                .exit
                                .as_ref()
                                .is_some_and(|exit| Some(exit) != evidence.exit.as_ref())
                        {
                            return Err(OperationError::EvidenceMismatch);
                        }
                        if let Some(expected) = &previous.incarnations {
                            let actual = evidence
                                .incarnations
                                .as_ref()
                                .ok_or(OperationError::EvidenceMismatch)?;
                            if expected.terminal_incarnation != actual.terminal_incarnation
                                || expected
                                    .process_incarnation
                                    .is_some_and(|value| Some(value) != actual.process_incarnation)
                            {
                                return Err(OperationError::EvidenceMismatch);
                            }
                        }
                        evidence.cancellation_requested |= previous.cancellation_requested;
                    }
                    row.entry.record.evidence = Some(evidence);
                }
                let write = CasWrite::new(
                    row.key.resource(self.context.identity.runtime_id()),
                    Version::new(row.version.get()),
                    Some(Payload::from(encode_entry(&row.entry)?)),
                );
                let mut commit = Commit::new(
                    OpId::new(uuid::Uuid::now_v7().to_string()),
                    D::from(OperationEvent::from_record(&row.entry.record)),
                    vec![ResourceId::Machine(self.context.identity.machine_id())],
                )
                .write(write);
                for write in completion.writes {
                    commit = commit.write(write);
                }
                for (data, affected) in completion.events {
                    commit = commit.event(data, affected);
                }
                Ok::<_, OperationError>(commit)
            })
            .map_err(|error| match error {
                OperationError::Event(source) => OperationError::Transition { op_id, source },
                other => other,
            })?;
        row.entry.record.updated_seq = cursor.seq;
        Ok(row.entry.record)
    }
}

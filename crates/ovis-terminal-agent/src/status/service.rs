//! Status reads, owning-process reports, clears and agent registrations.
//!
//! Every mutation is built inside the operation's commit unit from one snapshot, so the
//! sequence check, the row write, the receipt and the events are a single transaction.
//! A refusal commits the operation as failed with its typed error and nothing else.

use std::collections::BTreeMap;
use std::sync::Mutex;

use ovis_terminal_core::events::{EventError, ResourceId};
use ovis_terminal_core::ops::{Completion, OperationError};
use ovis_terminal_core::registry::{RuntimeId, TerminalId};
use ovis_terminal_protocol::domain::agent_status::IngestCounters;
use ovis_terminal_protocol::schema::ids::AgentId;
use ovis_terminal_protocol::schema::methods::agent::AgentResult;
use ovis_terminal_protocol::schema::methods::status::{StatusResult, TurnRecord};
use ovis_terminal_protocol::schema::records::AgentBinding;
use ovis_terminal_protocol::schema::scalar::{EventSeq, Revision};
use ovis_terminal_store::Snapshot;

use super::machine::Accepted;
use super::request::validated;
use super::row::{Location, Row, Stored};
use super::{AgentStatusEvent, Clear, ProcessOwner, ProcessTokens, Register, Report, StatusError};

/// Most child records one terminal keeps.
const CHILD_LIMIT: usize = 64;

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Tally {
    pub oversize: u64,
    pub malformed: u64,
    pub stale: u64,
}

/// Agent status of every terminal of one runtime.
#[derive(Debug)]
pub struct StatusService {
    pub(super) runtime: RuntimeId,
    pub(super) tokens: ProcessTokens,
    pub(super) tallies: Mutex<BTreeMap<TerminalId, Tally>>,
}

/// Sequence of the first event a unit built on `snapshot` commits.
pub(super) const fn next_event(snapshot: &Snapshot) -> EventSeq {
    EventSeq::new(snapshot.revision().get().saturating_add(1))
}

/// Sequence of the first domain event of an operation unit, which commits its
/// operation event first.
const fn operation_event(snapshot: &Snapshot) -> EventSeq {
    EventSeq::new(snapshot.revision().get().saturating_add(2))
}

/// The status event and, when a turn finished, its turn event.
pub(super) fn changes(
    row: &Row,
    ended: Option<TurnRecord>,
) -> (AgentStatusEvent, Option<AgentStatusEvent>) {
    (
        AgentStatusEvent::StatusChanged(Box::new(row.status.clone())),
        ended.map(|turn| AgentStatusEvent::TurnEnded(Box::new(turn))),
    )
}

/// Refuses a child record beyond the per-terminal bound.
pub(super) fn within_bounds(row: &Row, accepted: &Accepted) -> Result<(), StatusError> {
    match accepted.entry.report.id.as_deref() {
        Some(id)
            if !row.status.records.contains_key(id) && row.status.records.len() >= CHILD_LIMIT =>
        {
            Err(StatusError::InvalidRecord { field: "record.id" })
        }
        Some(_) | None => Ok(()),
    }
}

fn settle<D>(outcome: Result<Completion<D>, StatusError>) -> Result<Completion<D>, OperationError> {
    match outcome {
        Ok(completion) => Ok(completion),
        Err(StatusError::Encode(error)) => Err(error.into()),
        Err(StatusError::Event(error)) => Err(error.into()),
        Err(StatusError::Poisoned) => Err(EventError::Poisoned.into()),
        Err(
            refusal @ (StatusError::StaleSeq { .. }
            | StatusError::NotOwner
            | StatusError::StaleIncarnation
            | StatusError::InvalidRecord { .. }
            | StatusError::InvalidAgent
            | StatusError::AgentConflict),
        ) => Ok(Completion::failed(refusal.error_data())),
    }
}

impl StatusService {
    /// Status of the terminals of `runtime`, with no program bound yet.
    #[must_use]
    pub fn new(runtime: RuntimeId) -> Self {
        Self {
            runtime,
            tokens: ProcessTokens::default(),
            tallies: Mutex::new(BTreeMap::new()),
        }
    }

    /// The token registry programs authenticate their reports with.
    #[must_use]
    pub const fn tokens(&self) -> &ProcessTokens {
        &self.tokens
    }

    pub(super) const fn location(&self, terminal: TerminalId) -> Location {
        Location {
            runtime: self.runtime,
            terminal,
        }
    }

    pub(super) fn counters(&self, terminal: TerminalId) -> Result<IngestCounters, StatusError> {
        let tally = self
            .tallies
            .lock()
            .map_err(|_| StatusError::Poisoned)?
            .get(&terminal)
            .copied()
            .unwrap_or_default();
        Ok(IngestCounters::new(
            tally.oversize,
            tally.malformed,
            tally.stale,
        ))
    }

    /// Refuses `owner` unless it is the current program of its terminal.
    pub(super) fn require_current(&self, owner: ProcessOwner) -> Result<(), StatusError> {
        match self.tokens.current(owner.terminal)? {
            Some(current) if current == owner => Ok(()),
            Some(_) | None => Err(StatusError::StaleIncarnation),
        }
    }

    fn current(&self, snapshot: &Snapshot, terminal: TerminalId) -> Result<Stored, StatusError> {
        let process = self.tokens.current(terminal)?.map(|owner| owner.process);
        self.location(terminal).read(snapshot, process)
    }

    fn status_completion<D: From<AgentStatusEvent>>(
        &self,
        stored: &Stored,
        ended: Option<TurnRecord>,
    ) -> Result<Completion<D>, StatusError> {
        let row = &stored.row;
        let result = StatusResult::new(
            row.status.clone(),
            row.last_event,
            self.counters(stored.terminal)?,
        );
        let affected = vec![ResourceId::Terminal(stored.terminal)];
        let mut completion = Completion::new(serde_json::to_value(result)?).write(stored.write()?);
        let (status, turn) = changes(row, ended);
        for event in std::iter::once(status).chain(turn) {
            completion = completion.event(D::from(event), affected.clone());
        }
        Ok(completion)
    }

    /// Reads a terminal's status; a read never changes turn or finish facts.
    ///
    /// # Errors
    ///
    /// Returns a store or decoding failure.
    pub fn status<D: From<AgentStatusEvent>>(
        &self,
        snapshot: &Snapshot,
        terminal: TerminalId,
    ) -> Result<Completion<D>, OperationError> {
        settle(self.current(snapshot, terminal).and_then(|stored| {
            let row = stored.row;
            let result = StatusResult::new(row.status, row.last_event, self.counters(terminal)?);
            Ok(Completion::new(serde_json::to_value(result)?))
        }))
    }

    /// Reads a terminal's agent; a valid terminal without one has `agent: null`.
    ///
    /// # Errors
    ///
    /// Returns a store or decoding failure.
    pub fn agent<D: From<AgentStatusEvent>>(
        &self,
        snapshot: &Snapshot,
        terminal: TerminalId,
    ) -> Result<Completion<D>, OperationError> {
        settle(self.current(snapshot, terminal).and_then(|stored| {
            let revision = Revision::new(snapshot.revision().get());
            let result = AgentResult::new(stored.row.agent, revision);
            Ok(Completion::new(serde_json::to_value(result)?))
        }))
    }

    /// Applies an owning-process report whose sequence exceeds the accepted one.
    ///
    /// # Errors
    ///
    /// Refusals (`not_owner`, `invalid_record`, `stale_seq`) settle the operation as failed;
    /// a store or decoding failure is returned.
    pub fn report<D: From<AgentStatusEvent>>(
        &self,
        snapshot: &Snapshot,
        report: Report,
    ) -> Result<Completion<D>, OperationError> {
        settle((|| {
            let owner = self.tokens.authorize(&report.token, report.terminal)?;
            let seq = report.seq;
            let accepted = validated(report)?;
            let mut stored = self
                .location(owner.terminal)
                .read(snapshot, Some(owner.process))?;
            if let Some(current) = stored.row.status.report_seq
                && seq <= current
            {
                return Err(StatusError::StaleSeq { current });
            }
            within_bounds(&stored.row, &accepted)?;
            let ended = stored.row.accept(accepted, operation_event(snapshot));
            self.status_completion(&stored, ended)
        })())
    }

    /// Clears the root or one child record as the owning process; no turn ends.
    ///
    /// # Errors
    ///
    /// `not_owner` settles the operation as failed; a store or decoding failure is returned.
    pub fn clear<D: From<AgentStatusEvent>>(
        &self,
        snapshot: &Snapshot,
        clear: &Clear,
    ) -> Result<Completion<D>, OperationError> {
        settle((|| {
            let owner = self.tokens.authorize(&clear.token, clear.terminal)?;
            let mut stored = self
                .location(owner.terminal)
                .read(snapshot, Some(owner.process))?;
            stored
                .row
                .clear(clear.id.as_deref(), operation_event(snapshot));
            self.status_completion(&stored, None)
        })())
    }

    /// Registers an arbitrary label for the owning program's incarnation.
    ///
    /// # Errors
    ///
    /// Refusals (`not_owner`, `invalid_agent`, `agent_conflict`) settle the operation as
    /// failed; a store or decoding failure is returned.
    pub fn register<D: From<AgentStatusEvent>>(
        &self,
        snapshot: &Snapshot,
        register: Register,
    ) -> Result<Completion<D>, OperationError> {
        settle((|| {
            let owner = self.tokens.authorize(&register.token, register.terminal)?;
            if register.label.is_empty() || register.label.len() > super::request::NAME_LIMIT {
                return Err(StatusError::InvalidAgent);
            }
            let mut stored = self
                .location(owner.terminal)
                .read(snapshot, Some(owner.process))?;
            if stored.row.agent.is_some() {
                return Err(StatusError::AgentConflict);
            }
            let seq = operation_event(snapshot);
            let id = AgentId::new(format!("agent:{}", uuid::Uuid::now_v7()));
            let mut binding = AgentBinding::new(id.clone(), register.label, owner.process);
            binding.session.clone_from(&register.session);
            binding.revision = Revision::new(seq.get());
            stored.row.status.agent_id = Some(id);
            stored.row.status.session = register.session;
            stored.row.agent = Some(binding.clone());
            let result = AgentResult::new(Some(binding.clone()), binding.revision);
            Ok(Completion::new(serde_json::to_value(result)?)
                .write(stored.write()?)
                .event(
                    D::from(AgentStatusEvent::AgentRegistered(Box::new(binding))),
                    vec![ResourceId::Terminal(owner.terminal)],
                ))
        })())
    }
}

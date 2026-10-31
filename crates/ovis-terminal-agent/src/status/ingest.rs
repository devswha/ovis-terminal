//! OSC 7501 ingestion and exit invalidation, committed straight to the coordinator.
//!
//! An OSC record is authoritative only because it arrived in the output stream of the
//! terminal's own program; it takes the next producer sequence of that incarnation.

use ovis_terminal_core::events::{Commit, Coordinator, EventData, ResourceId, SnapshotProducer};
use ovis_terminal_core::registry::TerminalId;
use ovis_terminal_protocol::schema::methods::status::TurnRecord;
use ovis_terminal_protocol::schema::records::{StatusEntry, StatusOrigin, StatusSnapshot};
use ovis_terminal_protocol::schema::scalar::{ReportSeq, TimestampMs};
use ovis_terminal_store::OpId;
use ovis_terminal_vt::OscRecord;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::machine::Accepted;
use super::row::Stored;
use super::service::{Tally, changes, next_event, within_bounds};
use super::{
    AgentStatusEvent, OscOutcome, OscReport, ProcessOwner, StatusError, StatusService, classify,
};

/// One OSC record from the output stream of `owner`.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct OscOutput<'a> {
    /// The program whose output carried the record.
    pub owner: ProcessOwner,
    /// The record as the VT engine located it.
    pub record: &'a OscRecord,
    /// Time the record was read.
    pub at: TimestampMs,
}

impl<'a> OscOutput<'a> {
    /// A record read from `owner`'s output at `at`.
    #[must_use]
    pub const fn new(owner: ProcessOwner, record: &'a OscRecord, at: TimestampMs) -> Self {
        Self { owner, record, at }
    }
}

/// What ingesting one OSC record did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Ingested {
    /// The status changed; the committed snapshot.
    Applied(Box<StatusSnapshot>),
    /// An oversize or malformed 7501 record was counted and dropped.
    Dropped,
    /// Another OSC number or a query; nothing changed.
    Ignored,
}

fn unit<D: From<AgentStatusEvent>>(
    stored: &Stored,
    ended: Option<TurnRecord>,
) -> Result<Commit<D>, StatusError> {
    let affected = vec![ResourceId::Terminal(stored.terminal)];
    let mut events = changes(&stored.row, ended).into_iter();
    let first = events.next().ok_or(StatusError::Poisoned)?;
    let mut commit = Commit::new(
        OpId::new(uuid::Uuid::now_v7().to_string()),
        D::from(first),
        affected.clone(),
    )
    .write(stored.write()?);
    for event in events {
        commit = commit.event(D::from(event), affected.clone());
    }
    Ok(commit)
}

fn osc_entry(report: OscReport, seq: ReportSeq, at: TimestampMs) -> Accepted {
    let failed = report.failed_fields();
    let lifecycle = report.lifecycle;
    let mut entry = StatusEntry::new(report.report, StatusOrigin::Osc);
    entry.since_ms = at;
    entry.report_seq = seq;
    entry.raw = Some(report.provenance.raw.clone());
    entry.decode_errors = failed;
    entry.osc = Some(report.provenance);
    Accepted { entry, lifecycle }
}

impl StatusService {
    fn tally(&self, terminal: TerminalId, bump: fn(&mut Tally)) -> Result<Ingested, StatusError> {
        bump(
            self.tallies
                .lock()
                .map_err(|_| StatusError::Poisoned)?
                .entry(terminal)
                .or_default(),
        );
        Ok(Ingested::Dropped)
    }

    /// Applies, counts or ignores one OSC record of a program's output.
    ///
    /// # Errors
    ///
    /// Returns a store, decoding or coordinator failure; nothing was committed.
    pub fn ingest<D, P>(
        &self,
        coordinator: &Coordinator<D, P>,
        output: OscOutput<'_>,
    ) -> Result<Ingested, StatusError>
    where
        D: EventData + From<AgentStatusEvent> + Clone + Serialize + DeserializeOwned,
        P: SnapshotProducer,
    {
        let terminal = output.owner.terminal;
        let report = match classify(output.record) {
            OscOutcome::Report(report) => *report,
            OscOutcome::Oversize => {
                return self.tally(terminal, |t| t.oversize = t.oversize.saturating_add(1));
            }
            OscOutcome::Malformed => {
                return self.tally(terminal, |t| t.malformed = t.malformed.saturating_add(1));
            }
            OscOutcome::Query | OscOutcome::Foreign => return Ok(Ingested::Ignored),
        };
        let mut applied = None;
        let committed = coordinator.commit_unit(|snapshot| {
            let mut stored = self
                .location(terminal)
                .read(snapshot, Some(output.owner.process))?;
            let current = stored.row.status.report_seq.map_or(0, ReportSeq::get);
            let accepted = osc_entry(report, ReportSeq::new(current.saturating_add(1)), output.at);
            within_bounds(&stored.row, &accepted)?;
            let ended = stored.row.accept(accepted, next_event(snapshot));
            applied = Some(stored.row.status.clone());
            unit(&stored, ended)
        });
        match committed {
            Ok(_) => Ok(applied.map_or(Ingested::Ignored, |status| {
                Ingested::Applied(Box::new(status))
            })),
            Err(error) if error.is_rejection() => {
                self.tally(terminal, |t| t.malformed = t.malformed.saturating_add(1))
            }
            Err(error) => Err(error),
        }
    }

    /// Invalidates the records of `owner`'s exited program; a working root ends its turn.
    ///
    /// # Errors
    ///
    /// Returns a store, decoding or coordinator failure; nothing was committed.
    pub fn exited<D, P>(
        &self,
        coordinator: &Coordinator<D, P>,
        owner: ProcessOwner,
    ) -> Result<StatusSnapshot, StatusError>
    where
        D: EventData + From<AgentStatusEvent> + Clone + Serialize + DeserializeOwned,
        P: SnapshotProducer,
    {
        let mut status = None;
        coordinator.commit_unit(|snapshot| {
            let mut stored = self
                .location(owner.terminal)
                .read(snapshot, Some(owner.process))?;
            let ended = stored.row.exited(next_event(snapshot));
            status = Some(stored.row.status.clone());
            unit(&stored, ended)
        })?;
        status.ok_or(StatusError::Poisoned)
    }
}

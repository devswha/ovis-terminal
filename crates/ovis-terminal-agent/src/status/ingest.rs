//! OSC 7501 ingestion and exit invalidation, committed straight to the coordinator.
//!
//! An OSC record is authoritative only because it arrived in the output stream of the
//! terminal's own program; it takes the next producer sequence of that incarnation.

use ovis_terminal_core::events::{Commit, Coordinator, EventData, ResourceId, SnapshotProducer};
use ovis_terminal_core::registry::TerminalId;
use ovis_terminal_protocol::schema::methods::status::TurnRecord;
use ovis_terminal_protocol::schema::records::{StatusEntry, StatusOrigin};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Ingested {
    /// The status change was committed.
    Applied,
    /// The record was counted and dropped: oversize, malformed, from a program that
    /// is no longer current, or with no producer sequence left.
    Dropped,
    /// Another OSC number or a query; nothing changed.
    Ignored,
}

fn unit<D: From<AgentStatusEvent>>(
    stored: &Stored,
    ended: Option<TurnRecord>,
) -> Result<Commit<D>, StatusError> {
    let affected = vec![ResourceId::Terminal(stored.terminal)];
    let (status, turn) = changes(&stored.row, ended);
    let mut commit = Commit::new(
        OpId::new(uuid::Uuid::now_v7().to_string()),
        D::from(status),
        affected.clone(),
    )
    .write(stored.write()?);
    if let Some(turn) = turn {
        commit = commit.event(D::from(turn), affected);
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

const fn bump_oversize(tally: &mut Tally) {
    tally.oversize = tally.oversize.saturating_add(1);
}

const fn bump_malformed(tally: &mut Tally) {
    tally.malformed = tally.malformed.saturating_add(1);
}

const fn bump_stale(tally: &mut Tally) {
    tally.stale = tally.stale.saturating_add(1);
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
    /// Only the terminal's current program changes its status: a record from a replaced
    /// incarnation is counted as stale and dropped, as is a record after the producer
    /// sequence reached `u64::MAX`.
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
            OscOutcome::Oversize => return self.tally(terminal, bump_oversize),
            OscOutcome::Malformed => return self.tally(terminal, bump_malformed),
            OscOutcome::Query | OscOutcome::Foreign => return Ok(Ingested::Ignored),
        };
        let committed = coordinator.commit_unit(|snapshot| {
            self.require_current(output.owner)?;
            let mut stored = self
                .location(terminal)
                .read(snapshot, Some(output.owner.process))?;
            let current = stored.row.status.report_seq.unwrap_or(ReportSeq::new(0));
            let next = current
                .get()
                .checked_add(1)
                .ok_or(StatusError::StaleSeq { current })?;
            let accepted = osc_entry(report, ReportSeq::new(next), output.at);
            within_bounds(&stored.row, &accepted)?;
            let ended = stored.row.accept(accepted, next_event(snapshot));
            unit(&stored, ended)
        });
        match committed {
            Ok(_) => Ok(Ingested::Applied),
            Err(StatusError::StaleIncarnation | StatusError::StaleSeq { .. }) => {
                self.tally(terminal, bump_stale)
            }
            Err(error) if error.is_rejection() => self.tally(terminal, bump_malformed),
            Err(error) => Err(error),
        }
    }

    /// Invalidates the records of `owner`'s exited program; a working root ends its turn.
    ///
    /// # Errors
    ///
    /// Returns `stale_incarnation` without any change when `owner` is not the terminal's
    /// current program, or a store, decoding or coordinator failure.
    pub fn exited<D, P>(
        &self,
        coordinator: &Coordinator<D, P>,
        owner: ProcessOwner,
    ) -> Result<(), StatusError>
    where
        D: EventData + From<AgentStatusEvent> + Clone + Serialize + DeserializeOwned,
        P: SnapshotProducer,
    {
        coordinator.commit_unit(|snapshot| {
            self.require_current(owner)?;
            let mut stored = self
                .location(owner.terminal)
                .read(snapshot, Some(owner.process))?;
            let ended = stored.row.exited(next_event(snapshot));
            unit(&stored, ended)
        })?;
        Ok(())
    }
}

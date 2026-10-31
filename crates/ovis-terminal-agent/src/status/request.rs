//! Owning-process requests and their validation into accepted records.

use ovis_terminal_core::registry::TerminalId;
use ovis_terminal_protocol::schema::ids::ProcessToken;
use ovis_terminal_protocol::schema::records::{
    SessionRef, StatusEntry, StatusOrigin, StatusReport,
};
use ovis_terminal_protocol::schema::scalar::{ReportSeq, TimestampMs};

use super::StatusError;
use super::machine::{Accepted, Lifecycle};

/// Largest identifier and label, in bytes.
pub(super) const NAME_LIMIT: usize = 256;
/// Largest reported text field, in bytes; the OSC record bound.
const TEXT_LIMIT: usize = 4096;

/// A `status.report` by the process holding `token`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Report {
    /// Resolved target terminal.
    pub terminal: TerminalId,
    /// Token presented by the reporter.
    pub token: ProcessToken,
    /// Reported fields.
    pub record: StatusReport,
    /// Producer sequence.
    pub seq: ReportSeq,
    /// Acceptance time supplied by the boundary.
    pub at: TimestampMs,
}

impl Report {
    /// A report of `record` at `seq` by `token` for `terminal`, accepted at `at`.
    #[must_use]
    pub const fn new(terminal: TerminalId, token: ProcessToken, record: StatusReport) -> Self {
        Self {
            terminal,
            token,
            record,
            seq: ReportSeq::new(0),
            at: TimestampMs::new(0),
        }
    }
}

/// A `status.clear` of the root (no id) or one child record.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Clear {
    /// Resolved target terminal.
    pub terminal: TerminalId,
    /// Token presented by the caller.
    pub token: ProcessToken,
    /// Child record to clear.
    pub id: Option<String>,
}

impl Clear {
    /// Clears the root, or the child `id`, of `terminal`.
    #[must_use]
    pub const fn new(terminal: TerminalId, token: ProcessToken, id: Option<String>) -> Self {
        Self {
            terminal,
            token,
            id,
        }
    }
}

/// An `agent.register` with an arbitrary label.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Register {
    /// Resolved target terminal.
    pub terminal: TerminalId,
    /// Token presented by the registering program.
    pub token: ProcessToken,
    /// Arbitrary label; never matched against a brand list.
    pub label: String,
    /// Provider session, when declared.
    pub session: Option<SessionRef>,
}

impl Register {
    /// Registers `label` for the program holding `token`.
    #[must_use]
    pub const fn new(terminal: TerminalId, token: ProcessToken, label: String) -> Self {
        Self {
            terminal,
            token,
            label,
            session: None,
        }
    }
}

/// Parses a report once into a closed lifecycle state and bounded fields.
pub(super) fn validated(report: Report) -> Result<Accepted, StatusError> {
    let record = report.record;
    let lifecycle = Lifecycle::of(&record.state).ok_or(StatusError::InvalidRecord {
        field: "record.state",
    })?;
    let invalid = |field| StatusError::InvalidRecord { field };
    if record.progress.is_some_and(|progress| progress > 100) {
        return Err(invalid("record.progress"));
    }
    if record
        .id
        .as_deref()
        .is_some_and(|id| id.is_empty() || id.len() > NAME_LIMIT)
    {
        return Err(invalid("record.id"));
    }
    let texts = [
        ("record.kind", &record.kind),
        ("record.app", &record.app),
        ("record.title", &record.title),
        ("record.msg", &record.msg),
    ];
    if let Some((field, _)) = texts
        .iter()
        .find(|(_, text)| text.as_ref().is_some_and(|text| text.len() > TEXT_LIMIT))
    {
        return Err(invalid(field));
    }
    let mut entry = StatusEntry::new(record, StatusOrigin::Rpc);
    entry.since_ms = report.at;
    entry.report_seq = report.seq;
    Ok(Accepted { entry, lifecycle })
}

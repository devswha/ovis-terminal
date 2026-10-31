//! One write request: who writes, what, and the terminal state it is checked against.

use ovis_terminal_authz::Session;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::ClientId;
use ovis_terminal_protocol::schema::methods::input::{BracketedPolicy, Submit};
use ovis_terminal_protocol::schema::scalar::{ByteOffset, TimestampMs};

use super::Expected;
use crate::input::encoding::ModeSnapshot;
use crate::input::receipts::Reporter;
use crate::leases::{Caller, TerminalLeases, WriteKind};
use crate::registry::LeaseId;

/// What a request writes; every path enters the same sequence.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum Payload<'a> {
    /// Literal text and its submission as one entry (send and composer).
    Text {
        /// Literal text.
        text: &'a str,
        /// What follows it.
        submit: &'a Submit,
    },
    /// Pasted text.
    Paste {
        /// Literal text.
        text: &'a str,
        /// Bracketing policy.
        policy: BracketedPolicy,
    },
    /// Named keys.
    Keys(&'a [String]),
    /// Pre-encoded mouse reports.
    Mouse(&'a [u8]),
    /// Raw bytes; the sequencer asks the authority for the raw grant on its own terminal.
    Raw(&'a [u8]),
}

impl Payload<'_> {
    pub(super) const fn write_kind(&self) -> WriteKind {
        match self {
            Self::Raw(_) => WriteKind::Raw,
            Self::Text { .. } | Self::Paste { .. } | Self::Keys(_) | Self::Mouse(_) => {
                WriteKind::Input
            }
        }
    }
}

/// An authenticated session writing as one client label at one instant.
#[derive(Debug, Clone, Copy)]
pub struct Writer<'a> {
    pub(super) session: &'a Session,
    client: &'a ClientId,
    pub(super) at: TimestampMs,
    pub(super) lease: Option<LeaseId>,
}

impl<'a> Writer<'a> {
    /// A writer presenting no lease.
    pub const fn new(session: &'a Session, client: &'a ClientId, at: TimestampMs) -> Self {
        Self {
            session,
            client,
            at,
            lease: None,
        }
    }

    /// The writer presents the interact lease `lease`.
    #[must_use]
    pub const fn presenting(mut self, lease: LeaseId) -> Self {
        self.lease = Some(lease);
        self
    }

    pub(super) const fn caller(&self) -> Caller<'a> {
        Caller::new(self.session, self.client, self.at)
    }
}

/// One operation's input and the identities it was resolved against.
#[derive(Debug, Clone)]
pub struct Submission<'a> {
    pub(super) op_id: OpId,
    pub(super) expected: Expected,
    pub(super) payload: Payload<'a>,
    pub(super) reporter: Reporter,
}

impl<'a> Submission<'a> {
    /// Submits `payload` under `op_id`; agent stages stay unknown.
    pub const fn new(op_id: OpId, expected: Expected, payload: Payload<'a>) -> Self {
        Self {
            op_id,
            expected,
            payload,
            reporter: Reporter::Undeclared,
        }
    }

    /// A declared producer may report agent stages for this entry.
    #[must_use]
    pub const fn reported_by(mut self, reporter: Reporter) -> Self {
        self.reporter = reporter;
        self
    }
}

/// A writer and its submission.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    pub(super) writer: Writer<'a>,
    pub(super) submission: Submission<'a>,
}

impl<'a> Request<'a> {
    /// Pairs `writer` with `submission`.
    pub const fn new(writer: Writer<'a>, submission: Submission<'a>) -> Self {
        Self { writer, submission }
    }
}

/// The instant of a check and the output offset reached by then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Moment {
    /// Instant of the check.
    pub at: TimestampMs,
    /// Raw output offset the terminal had reached.
    pub output: ByteOffset,
}

impl Moment {
    /// The check at `at` with output at `output`.
    pub const fn new(at: TimestampMs, output: ByteOffset) -> Self {
        Self { at, output }
    }
}

/// The terminal state an admission or a write boundary is checked against.
#[derive(Debug, Clone, Copy)]
pub struct View<'a> {
    pub(super) leases: &'a TerminalLeases,
    pub(super) modes: &'a ModeSnapshot,
    pub(super) moment: Moment,
}

impl<'a> View<'a> {
    /// Current leases and modes at `moment`.
    pub const fn new(leases: &'a TerminalLeases, modes: &'a ModeSnapshot, moment: Moment) -> Self {
        Self {
            leases,
            modes,
            moment,
        }
    }
}

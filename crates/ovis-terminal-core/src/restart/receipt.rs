//! What a restart preserved, lost and interrupted.

use ovis_terminal_protocol::domain::restart::{InterruptedOp, InterruptedWait};
use ovis_terminal_protocol::schema::ids::ServerBootId as WireBootId;
use ovis_terminal_protocol::schema::methods::server::ServerTransitionResult;
use ovis_terminal_protocol::schema::scalar::{EventSeq, Incarnation};
use serde::{Deserialize, Serialize};

use crate::registry::{ProcessIdentity, RuntimeId, ServerBootId, TerminalGeneration, TerminalId};

const fn incarnation(process: &ProcessIdentity) -> Incarnation {
    Incarnation::new(process.generation().get())
}

/// A program incarnation the holder owned when the restart began.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Preserved {
    /// Terminal identity.
    pub terminal: TerminalId,
    /// Terminal incarnation.
    pub terminal_incarnation: TerminalGeneration,
    /// The process: incarnation, PID and start token.
    pub process: ProcessIdentity,
}

impl Preserved {
    /// Names `process` running in `terminal` at `terminal_incarnation`.
    #[must_use]
    pub const fn new(
        terminal: TerminalId,
        terminal_incarnation: TerminalGeneration,
        process: ProcessIdentity,
    ) -> Self {
        Self {
            terminal,
            terminal_incarnation,
            process,
        }
    }

    const fn wire(&self) -> Incarnation {
        incarnation(&self.process)
    }
}

/// The outcome of one completed restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RestartReceipt {
    /// Runtime lifetime, unchanged.
    pub runtime: RuntimeId,
    /// Boot identity replaced.
    pub from: ServerBootId,
    /// Boot identity of the successor.
    pub to: ServerBootId,
    /// Incarnations the successor proved with the same PID and start token.
    pub preserved: Vec<Preserved>,
    /// Incarnations held before the boundary that the successor did not find.
    pub lost: Vec<Preserved>,
    /// Operations in flight at the boundary, classified from the ledger.
    pub interrupted_ops: Vec<InterruptedOp>,
    /// Waits armed at the boundary; each ended `cancelled`.
    pub interrupted_waits: Vec<InterruptedWait>,
    /// Latest committed sequence when the intent was persisted.
    pub seq_before: EventSeq,
    /// Latest committed sequence once the successor recovered.
    pub seq_after: EventSeq,
    /// The restart was finished by a later open after a crash at a barrier.
    pub resumed: bool,
}

impl RestartReceipt {
    /// The published `server.restart` result.
    #[must_use]
    pub fn transition(&self) -> ServerTransitionResult {
        let mut result = ServerTransitionResult::restarted(
            (
                WireBootId::new(self.from.to_string()),
                WireBootId::new(self.to.to_string()),
            ),
            self.preserved.iter().map(Preserved::wire).collect(),
            self.interrupted_ops
                .iter()
                .map(|op| op.op_id.clone())
                .collect(),
        );
        result.ended_incarnations = self.lost.iter().map(Preserved::wire).collect();
        result
    }
}

/// The outcome of one explicit stop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct StopReceipt {
    /// Boot identity that stopped.
    pub from: ServerBootId,
    /// Incarnations left running in their holder; none was signalled.
    pub preserved: Vec<ProcessIdentity>,
    /// Incarnations the stop ended, each with reaped exit evidence.
    pub ended: Vec<ProcessIdentity>,
    /// Launches whose process identity was still pending at the stop.
    pub pending: Vec<TerminalId>,
}

impl StopReceipt {
    /// A stop of `from` that left `preserved` running and ended `ended`.
    #[must_use]
    pub fn new(
        from: ServerBootId,
        (preserved, ended): (Vec<ProcessIdentity>, Vec<ProcessIdentity>),
        pending: Vec<TerminalId>,
    ) -> Self {
        Self {
            from,
            preserved,
            ended,
            pending,
        }
    }

    /// The published `server.stop` result.
    #[must_use]
    pub fn transition(&self) -> ServerTransitionResult {
        ServerTransitionResult::stopped(
            WireBootId::new(self.from.to_string()),
            self.preserved.iter().map(incarnation).collect(),
            self.ended.iter().map(incarnation).collect(),
        )
    }
}

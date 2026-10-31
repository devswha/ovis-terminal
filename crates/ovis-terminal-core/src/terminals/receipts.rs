//! Lifecycle receipts name exactly which process incarnations were affected.

use ovis_terminal_holder::protocol::Exit;
use ovis_terminal_protocol::schema::scalar::DurationMs;
use serde::{Deserialize, Serialize};

use crate::registry::{ProcessIdentity, TerminalGeneration, TerminalId};

/// Explicit stages of program termination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum KillStage {
    /// SIGHUP delivered to the held program's process group.
    Hangup,
    /// SIGTERM delivered to that same group.
    Terminate,
    /// SIGKILL delivered to that same group.
    Kill,
}

/// A kill bound to a full process identity, never a PID alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct KillRequest {
    /// Explicit terminal.
    pub terminal: TerminalId,
    /// Process identity that must still match at every signal boundary.
    pub expected: ProcessIdentity,
    /// Grace after SIGHUP.
    pub hup_ms: DurationMs,
    /// Grace after SIGTERM.
    pub term_ms: DurationMs,
}

impl KillRequest {
    /// Parses a closed kill request, rejecting numeric identity counters.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` with the original deserializer failure.
    pub fn decode(bytes: &[u8]) -> Result<Self, super::TerminalError> {
        serde_json::from_slice(bytes).map_err(super::TerminalError::InvalidJson)
    }

    /// Requests immediate staged escalation for a known process.
    pub const fn immediate(terminal: TerminalId, expected: ProcessIdentity) -> Self {
        Self {
            terminal,
            expected,
            hup_ms: DurationMs::new(0),
            term_ms: DurationMs::new(0),
        }
    }
}

/// Delivered signals and reaped exit evidence are independent facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct KillReceipt {
    /// Target terminal.
    pub terminal: TerminalId,
    /// Target terminal incarnation.
    pub terminal_incarnation: TerminalGeneration,
    /// Exact process affected.
    pub process: ProcessIdentity,
    /// Only stages the kernel accepted.
    pub delivered: Vec<KillStage>,
    /// Absent means that reaped exit was not observed, even if a signal was delivered.
    pub exit: Option<Exit>,
}

/// Metadata removal neither implies nor performs termination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RemovalReceipt {
    /// Tombstoned terminal identity.
    pub terminal: TerminalId,
    /// Historical terminal incarnation.
    pub terminal_incarnation: TerminalGeneration,
    /// Historical processes whose metadata was removed; no signal is sent to them.
    pub affected: Vec<ProcessIdentity>,
}

/// Control ownership is independent of the program's lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ControlState {
    /// A control actor is consuming holder evidence.
    Running,
    /// Control stopped without signalling programs.
    Stopped,
    /// Control released ownership for a replacement core.
    Handover,
}

/// Administrative scope and its actual process effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ControlReceipt {
    /// The control link stopped; the listed programs were not signalled.
    Stopped {
        /// Incarnations left in holder ownership.
        preserved: Vec<ProcessIdentity>,
        /// Admitted launches whose process identity was still pending.
        pending: Vec<TerminalId>,
    },
    /// Ownership was released for restart; this does not claim a new core boot succeeded.
    HandoverPrepared {
        /// Incarnations left in holder ownership.
        preserved: Vec<ProcessIdentity>,
        /// Admitted launches still pending.
        pending: Vec<TerminalId>,
    },
    /// Explicit program-stop results; control remains available.
    ProgramStop {
        /// Incarnations with actual holder exit evidence.
        ended: Vec<ProcessIdentity>,
        /// Incarnations whose exit remained unproven.
        unknown: Vec<ProcessIdentity>,
        /// Launches whose process identity remained pending at the stop boundary.
        pending: Vec<TerminalId>,
    },
}

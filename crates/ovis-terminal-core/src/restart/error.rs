//! The restart boundary error.

use ovis_terminal_protocol::domain::restart::RestartPhase;

use crate::events::EventError;
use crate::ops::OperationError;
use crate::registry::{HostBootId, IdentityError, ProcessIdentity, RuntimeId};
use crate::terminals::TerminalError;

/// Every refusal or failure of a restart, a stop or work offered during one.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RestartError {
    /// Readiness is withdrawn while a restart runs.
    #[error("the control server is between boots (phase {phase:?})")]
    Restarting {
        /// The phase reached so far.
        phase: RestartPhase,
    },
    /// Control was stopped; no lifecycle actor serves requests.
    #[error("control was stopped and serves no request")]
    ControlStopped,
    /// The restart intent could not be made durable or read back.
    #[error("the restart intent could not be {operation}")]
    IntentIo {
        /// What failed.
        operation: &'static str,
        /// The I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The restart intent on disk is not of the published shape.
    #[error("the restart intent on disk does not have the published shape")]
    IntentMalformed(#[source] serde_json::Error),
    /// The intent was written under another host boot: no process of it can have survived.
    #[error("the restart intent names host boot {recorded}, but this host booted as {current}")]
    HostRebooted {
        /// Host boot the intent recorded.
        recorded: HostBootId,
        /// Host boot now.
        current: HostBootId,
    },
    /// The intent belongs to another runtime lifetime.
    #[error("the restart intent names runtime {recorded}, not {current}")]
    LifetimeMismatch {
        /// Runtime the intent recorded.
        recorded: RuntimeId,
        /// Runtime asked to open.
        current: RuntimeId,
    },
    /// Programs were signalled but the exit of some was not observed.
    #[error("{} program(s) were signalled but their exit was not observed", .unknown.len())]
    StopUnproven {
        /// Incarnations with reaped exit evidence.
        ended: Vec<ProcessIdentity>,
        /// Incarnations whose exit stayed unproven.
        unknown: Vec<ProcessIdentity>,
    },
    /// The lifecycle actor refused or failed.
    #[error("the terminal lifecycle refused the step")]
    Terminal(#[from] TerminalError),
    /// The operation ledger refused or failed.
    #[error("the operation ledger refused the step")]
    Operation(#[from] OperationError),
    /// Reading committed state failed.
    #[error("committed state could not be read")]
    Events(#[from] EventError),
    /// The successor's store could not be opened.
    #[error("the lifecycle store could not be opened")]
    Store(#[from] ovis_terminal_store::StoreError),
    /// An identity could not be minted.
    #[error("the next boot identity could not be minted")]
    Identity(#[from] IdentityError),
    /// The wall clock lies before the Unix epoch.
    #[error("the wall clock lies before the Unix epoch")]
    Clock(#[source] std::time::SystemTimeError),
    /// The wall clock does not fit a millisecond timestamp.
    #[error("the wall clock does not fit a millisecond timestamp")]
    ClockRange(#[source] std::num::TryFromIntError),
    /// A thread panicked while it held the control state.
    #[error("the control state lock was poisoned by a panicking thread")]
    Poisoned,
}

impl RestartError {
    /// Stable machine token.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Restarting { .. } => "restarting",
            Self::ControlStopped => "control_stopped",
            Self::IntentIo { .. } => "persistence_failed",
            Self::IntentMalformed(_) => "intent_malformed",
            Self::HostRebooted { .. } => "host_rebooted",
            Self::LifetimeMismatch { .. } => "lifetime_mismatch",
            Self::StopUnproven { .. } => "outcome_unknown",
            Self::Terminal(error) => error.kind(),
            Self::Operation(error) => error.kind(),
            Self::Events(error) => error.kind(),
            Self::Store(error) => error.kind(),
            Self::Identity(error) => error.kind(),
            Self::Clock(_) | Self::ClockRange(_) => "clock_unrepresentable",
            Self::Poisoned => "restart_poisoned",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Restarting { .. } => {
                "wait for the restart boundary to complete (server.recovery), then retry"
            }
            Self::ControlStopped => "start the control server explicitly with server.start",
            Self::IntentIo { .. } => "check the state directory; the restart did not proceed",
            Self::IntentMalformed(_) => {
                "inspect the restart intent file; it is kept, never replaced by a default"
            }
            Self::HostRebooted { .. } => {
                "treat every terminal as restored metadata; relaunch explicitly"
            }
            Self::LifetimeMismatch { .. } => {
                "open the runtime that wrote the intent; lifetimes are never inferred"
            }
            Self::StopUnproven { .. } => {
                "inspect process evidence of the listed incarnations before relaunching"
            }
            Self::Terminal(error) => error.hint(),
            Self::Operation(error) => error.hint(),
            Self::Events(error) => error.hint(),
            Self::Store(error) => error.hint(),
            Self::Identity(error) => error.hint(),
            Self::Clock(_) | Self::ClockRange(_) => "correct the runtime clock",
            Self::Poisoned => "restart the runtime; a control thread panicked",
        }
    }
}

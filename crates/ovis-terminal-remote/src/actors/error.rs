//! Failures of a call routed to one machine's actor.

use std::time::Duration;

use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::scalar::EventSeq;

/// Why a call to a machine did not produce the remote's answer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ActorError {
    /// No actor serves this machine.
    #[error("no actor serves {}", machine.as_str())]
    NotFound {
        /// The machine.
        machine: MachineId,
    },
    /// The machine already has an actor.
    #[error("{} already has an actor", machine.as_str())]
    Exists {
        /// The machine.
        machine: MachineId,
    },
    /// The machine's queue is full; nothing was queued.
    #[error("the queue of {} holds its {capacity} calls", machine.as_str())]
    QueueFull {
        /// The machine.
        machine: MachineId,
        /// The queue bound.
        capacity: usize,
    },
    /// The remote did not answer within the call's deadline; it was told to cancel.
    #[error("{} did not answer within {deadline:?}", machine.as_str())]
    DeadlineExceeded {
        /// The machine.
        machine: MachineId,
        /// The call's deadline.
        deadline: Duration,
    },
    /// The requested deadline is zero or above the configured bound.
    #[error("the deadline {requested:?} is outside 1 ms to {max:?}")]
    DeadlineOutOfRange {
        /// The requested deadline.
        requested: Duration,
        /// The bound.
        max: Duration,
    },
    /// The machine has no line; its resources keep their last-known state.
    #[error("{} is unreachable", machine.as_str())]
    Unreachable {
        /// The machine.
        machine: MachineId,
        /// Last remote sequence mirrored before the loss.
        last_seen_seq: Option<EventSeq>,
    },
    /// The machine's actor stopped.
    #[error("the actor of {} stopped", machine.as_str())]
    Stopped {
        /// The machine.
        machine: MachineId,
    },
    /// The request could not be encoded.
    #[error("the request could not be encoded")]
    Encode(#[source] serde_json::Error),
}

impl ActorError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "not_found",
            Self::Exists { .. } => "machine_exists",
            Self::QueueFull { .. } => "backpressure",
            Self::DeadlineExceeded { .. } => "deadline_exceeded",
            Self::DeadlineOutOfRange { .. } => "invalid_params",
            Self::Unreachable { .. } => "machine_unreachable",
            Self::Stopped { .. } => "unavailable",
            Self::Encode(_) => "internal",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "list machines to find a live id",
            Self::Exists { .. } => "attach a new line to the existing actor instead",
            Self::QueueFull { .. } => "retry after calls to this machine settle",
            Self::DeadlineExceeded { .. } => {
                "inspect the remote operation before retrying; other machines are unaffected"
            }
            Self::DeadlineOutOfRange { .. } => "pass a deadline within the advertised bound",
            Self::Unreachable { .. } => "reconnect the machine; nothing ran locally instead",
            Self::Stopped { .. } => "restart the runtime that owns the machine actors",
            Self::Encode(_) => "send JSON-encodable parameters",
        }
    }

    /// Whether a later identical call may succeed.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(
            self,
            Self::QueueFull { .. } | Self::DeadlineExceeded { .. } | Self::Unreachable { .. }
        )
    }
}

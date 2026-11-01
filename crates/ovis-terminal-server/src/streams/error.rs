//! The attach-stream boundary error: stable kinds, recovery hints and sources.

use ovis_terminal_core::events::EventError;
use ovis_terminal_core::leases::LeaseError;
use ovis_terminal_core::registry::TargetResolutionError;
use ovis_terminal_core::screen::ScreenError;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use thiserror::Error;

use super::credit::CreditRefusal;

/// Why an attach or stream operation was refused; nothing was delivered for it.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StreamError {
    /// No open stream with this identity belongs to the caller.
    #[error("no open stream with this identity belongs to the caller")]
    StreamNotFound,
    /// The acknowledgement was refused and granted no credit.
    #[error("the acknowledgement was refused")]
    InvalidCredit(#[source] CreditRefusal),
    /// A CREDIT frame did not carry an eight-byte offset.
    #[error("a CREDIT payload has {len} bytes, not eight")]
    MalformedCredit {
        /// Payload length received.
        len: usize,
    },
    /// The client sent terminal bytes on a download-only stream channel.
    #[error("channel {channel} carries output only; nothing was written to the program")]
    DirectionViolation {
        /// The stream channel.
        channel: u32,
    },
    /// Channel streams were requested on an endpoint that routes none.
    #[error("this endpoint delivers attach streams as notifications only")]
    NeedsFramed,
    /// The terminal serves its maximum number of streams.
    #[error("the terminal already serves {limit} streams")]
    StreamLimit {
        /// Streams one terminal serves.
        limit: usize,
    },
    /// The terminal changed incarnation since the caller resolved it.
    #[error("the terminal is no longer the incarnation the caller named")]
    StaleTarget,
    /// The terminal has no stream service.
    #[error("the terminal has no attach stream service")]
    NotAttachable,
    /// The operation id was used before with different parameters.
    #[error("the operation id was used with different parameters")]
    OpReused,
    /// The target did not resolve to one terminal.
    #[error("the target did not resolve")]
    Target(#[from] TargetResolutionError),
    /// The lease table refused the change.
    #[error("the lease change was refused")]
    Lease(#[from] LeaseError),
    /// The screen pipeline refused the output or read.
    #[error("the screen pipeline refused the request")]
    Screen(#[from] ScreenError),
    /// A lease transition could not be committed.
    #[error("the lease transition was not committed")]
    Journal(#[from] EventError),
    /// The request did not decode or the reply did not encode.
    #[error("the payload is not valid JSON for this method")]
    Payload(#[from] serde_json::Error),
    /// A blocking stream step did not complete.
    #[error("a blocking stream step did not complete")]
    Join(#[from] tokio::task::JoinError),
    /// The system clock is before the Unix epoch or past its range.
    #[error("the system clock is outside the representable range")]
    Clock,
    /// A state lock is poisoned by an earlier panic.
    #[error("stream state is unavailable after an internal panic")]
    Poisoned,
}

impl StreamError {
    /// The stable machine-readable kind clients branch on.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::StreamNotFound => "stream_not_found",
            Self::InvalidCredit(_) | Self::MalformedCredit { .. } => "invalid_credit",
            Self::DirectionViolation { .. } => "direction_violation",
            Self::NeedsFramed => "needs_framed",
            Self::StreamLimit { .. } => "stream_limit",
            Self::StaleTarget => "stale_target",
            Self::NotAttachable => "not_found",
            Self::OpReused => "op_reused",
            Self::Target(source) => source.kind(),
            Self::Lease(source) => source.kind(),
            Self::Screen(source) => source.kind(),
            Self::Journal(source) => source.kind(),
            Self::Payload(_) => "invalid_params",
            Self::Join(_) | Self::Clock | Self::Poisoned => "internal",
        }
    }

    /// Recovery guidance for the kind.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::StreamNotFound => "use a stream_id this principal opened and has not closed",
            Self::InvalidCredit(refusal) => refusal.hint(),
            Self::MalformedCredit { .. } => {
                "send the cumulative offset as eight little-endian bytes"
            }
            Self::DirectionViolation { .. } => {
                "send input through input.* RPCs under an interact lease"
            }
            Self::NeedsFramed => "request stream_mode plain and read attach.data notifications",
            Self::StreamLimit { .. } => "close an unused stream on this terminal, then retry",
            Self::StaleTarget => "resolve the terminal again and attach to its current incarnation",
            Self::NotAttachable => "attach to a terminal that is running under this runtime",
            Self::OpReused => "mint a fresh op_id for a different request",
            Self::Target(source) => source.hint(),
            Self::Lease(source) => source.hint(),
            Self::Screen(source) => source.hint(),
            Self::Journal(source) => source.hint(),
            Self::Payload(_) => "send params that match the method schema",
            Self::Join(_) | Self::Poisoned => "restart the control server; terminals keep running",
            Self::Clock => "correct the host clock, then retry",
        }
    }

    const fn retryable(&self) -> bool {
        matches!(self, Self::StreamLimit { .. } | Self::Journal(_))
    }

    /// The typed wire error.
    #[must_use]
    pub fn boundary(&self) -> BoundaryError {
        let refusal =
            BoundaryError::domain(ErrorKind::from(self.kind().to_owned()), self.retryable())
                .with_hint(self.hint());
        match self {
            Self::Lease(LeaseError::EpochMismatch { current, .. }) => {
                refusal.with_current_lease(super::records::lease_record(current))
            }
            _ => refusal,
        }
    }
}

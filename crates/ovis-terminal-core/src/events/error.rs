//! Errors at the coordinator boundary.

use ovis_terminal_store::StoreError;

/// A failed commit, snapshot, subscription or page operation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EventError {
    /// The durable store rejected the operation.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// A typed record could not be encoded or decoded.
    #[error("the event or snapshot record could not be encoded or decoded")]
    Serialization(#[from] serde_json::Error),
    /// An accepted JSON cursor is malformed.
    #[error("the cursor must contain valid identities and a canonical decimal sequence")]
    InvalidParams(#[source] serde_json::Error),
    /// A durable event payload cannot be decoded.
    #[error("the durable event payload cannot be decoded")]
    DecodeRecord(#[source] serde_json::Error),
    /// A previous coordinator operation panicked.
    #[error("the coordinator stopped while holding its commit boundary")]
    Poisoned,
    /// A page no longer has a pinned read transaction.
    #[error("the requested snapshot page is no longer pinned")]
    StaleSnapshot,
    /// An offset is outside the pinned representation.
    #[error("the page offset is outside the snapshot")]
    InvalidOffset,
    /// The requested cursor is newer than the committed head.
    #[error("the cursor is ahead of the committed state")]
    SequenceAhead,
    /// No producer can supply the requested snapshot.
    #[error("the snapshot producer cannot read its committed section")]
    SnapshotUnavailable,
    /// The configured bound is exhausted.
    #[error("the coordinator admission bound has been reached")]
    LimitExceeded,
    /// The mutation does not name an affected resource.
    #[error("a committed event must name its affected resources")]
    MissingResources,
    /// A bounded delivery queue overflowed.
    #[error("the subscription lost delivery and requires a fresh snapshot")]
    Missed,
    /// No event was received before the caller's timeout.
    #[error("no committed event arrived before the receive deadline")]
    Receive(#[source] std::sync::mpsc::RecvTimeoutError),
    /// A nonblocking delivery has no event or was disconnected.
    #[error("no committed event is available on this delivery channel")]
    TryReceive(#[source] std::sync::mpsc::TryRecvError),
    /// A stored event does not match its declared kind or identity.
    #[error("the durable event record does not match its envelope")]
    InvalidRecord,
}

impl EventError {
    /// Stable machine-readable failure token.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Store(error) => error.kind(),
            Self::InvalidParams(_) | Self::InvalidOffset | Self::MissingResources => {
                "invalid_params"
            }
            Self::Serialization(_) => "persistence_failed",
            Self::Poisoned => "coordinator_stopped",
            Self::StaleSnapshot => "stale_snapshot",
            Self::SequenceAhead => "seq_ahead",
            Self::SnapshotUnavailable => "snapshot_unavailable",
            Self::LimitExceeded => "limit_exceeded",
            Self::Missed => "missed",
            Self::Receive(_) | Self::TryReceive(_) => "delivery_unavailable",
            Self::InvalidRecord | Self::DecodeRecord(_) => "invalid_event",
        }
    }

    /// Recovery guidance independent of display text.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Store(error) => error.hint(),
            Self::InvalidParams(_) | Self::InvalidOffset | Self::MissingResources => {
                "supply a valid typed record and a page offset within the returned total"
            }
            Self::Serialization(_) => "inspect the failed typed record serialization",
            Self::Poisoned => "restart the core and recover from the durable log",
            Self::StaleSnapshot | Self::Missed | Self::SequenceAhead => {
                "start a fresh snapshot subscription"
            }
            Self::SnapshotUnavailable => "wait for the owning producer to become available",
            Self::LimitExceeded => "release a pinned snapshot or disconnect an unused subscription",
            Self::Receive(_) | Self::TryReceive(_) => {
                "resume from the last processed cursor if the connection closed"
            }
            Self::InvalidRecord | Self::DecodeRecord(_) => {
                "inspect the durable event record before resuming delivery"
            }
        }
    }
}

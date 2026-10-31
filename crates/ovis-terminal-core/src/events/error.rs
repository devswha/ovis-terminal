//! Errors at the coordinator boundary.

use ovis_terminal_store::StoreError;

use super::{PageIndex, SnapshotTornReason};

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
    /// A page sequence is missing, duplicated, reordered or incomplete.
    #[error("the snapshot page sequence is incomplete at page {expected_page:?}")]
    SnapshotTorn {
        /// First page index required to continue reconstruction.
        expected_page: PageIndex,
        /// Page index received instead, when one was supplied.
        received_page: Option<PageIndex>,
        /// Structural reason reconstruction was rejected.
        reason: SnapshotTornReason,
    },
    /// Complete bytes matched no valid typed snapshot.
    #[error("the completed snapshot bytes are not a valid typed snapshot")]
    SnapshotDecode {
        /// Final page involved in the invalid representation.
        page: PageIndex,
        /// Typed decoding failure retained as the source.
        #[source]
        source: serde_json::Error,
    },
    /// The requested cursor is newer than the committed head.
    #[error("the cursor is ahead of the committed state")]
    SequenceAhead,
    /// No producer can supply the requested snapshot.
    #[error("the snapshot producer cannot read its committed section")]
    SnapshotUnavailable,
    /// The configured bound is exhausted.
    #[error("the coordinator admission bound has been reached")]
    LimitExceeded,
    /// A request waiting for a pinned-snapshot slot was withdrawn before its turn.
    #[error("the queued snapshot request was withdrawn before its turn")]
    AdmissionWithdrawn,
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
            Self::SnapshotTorn { .. } | Self::SnapshotDecode { .. } => "snapshot_torn",
            Self::SequenceAhead => "seq_ahead",
            Self::SnapshotUnavailable => "snapshot_unavailable",
            Self::LimitExceeded => "limit_exceeded",
            Self::AdmissionWithdrawn => "unavailable",
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
            Self::SnapshotTorn { .. } | Self::SnapshotDecode { .. } => {
                "request the reported missing page index from a fresh pinned snapshot"
            }
            Self::SnapshotUnavailable => "wait for the owning producer to become available",
            Self::LimitExceeded => "release a pinned snapshot or disconnect an unused subscription",
            Self::AdmissionWithdrawn => "connect to an available runtime and inspect the operation",
            Self::Receive(_) | Self::TryReceive(_) => {
                "resume from the last processed cursor if the connection closed"
            }
            Self::InvalidRecord | Self::DecodeRecord(_) => {
                "inspect the durable event record before resuming delivery"
            }
        }
    }

    /// Page index at which checked snapshot reconstruction stopped.
    pub const fn snapshot_page_index(&self) -> Option<PageIndex> {
        match self {
            Self::SnapshotTorn { expected_page, .. } => Some(*expected_page),
            Self::SnapshotDecode { page, .. } => Some(*page),
            _ => None,
        }
    }

    /// Structural torn-snapshot reason, when this is a reconstruction failure.
    pub const fn snapshot_torn_reason(&self) -> Option<SnapshotTornReason> {
        match self {
            Self::SnapshotTorn { reason, .. } => Some(*reason),
            Self::SnapshotDecode { .. } => Some(SnapshotTornReason::InvalidSnapshot),
            _ => None,
        }
    }
}

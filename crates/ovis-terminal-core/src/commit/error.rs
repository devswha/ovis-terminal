//! Typed failures of the commit boundary and its snapshot pager.

use ovis_terminal_protocol::schema::scalar::{EventSeq, TimestampMs};
use ovis_terminal_store::StoreError;

use crate::events::EventError;

/// Why a commit, page or release did not happen; nothing failed is acknowledged.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CommitError {
    /// The coordinator or its store refused.
    #[error("the commit coordinator refused")]
    Event(#[from] EventError),
    /// The store ran out of space; the transaction rolled back.
    #[error("the store has no space for the transaction")]
    StorageFull {
        /// The store's refusal.
        #[source]
        source: EventError,
    },
    /// The upstream position was already committed, or is older than one that was.
    #[error("upstream sequence {} is not after the committed {}", .upstream.get(), .last.get())]
    RemoteReplayed {
        /// Upstream sequence offered.
        upstream: EventSeq,
        /// Highest upstream sequence already committed for that lifetime.
        last: EventSeq,
    },
    /// The stored upstream high-water mark could not be read or written.
    #[error("the upstream high-water mark is not a decimal sequence")]
    RemoteMark {
        /// The decoding failure.
        #[source]
        source: serde_json::Error,
    },
    /// The store reported a resource state this build does not know.
    #[error("the upstream high-water mark has an unknown state")]
    RemoteMarkUnknown,
    /// The pinned snapshot's lease ended at its advertised expiry.
    #[error("the snapshot lease expired at {}", .expired_ms.get())]
    PageExpired {
        /// The advertised expiry.
        expired_ms: TimestampMs,
    },
    /// Every bounded writer submission slot is taken.
    #[error("all {slots} writer submission slots are in use")]
    Backpressure {
        /// Configured slots.
        slots: usize,
    },
    /// A bound was zero or an expiry overflowed.
    #[error("the {bound} bound is invalid")]
    InvalidBound {
        /// Which bound.
        bound: &'static str,
    },
    /// The blocking commit worker did not return.
    #[error("the blocking commit worker stopped")]
    Worker(#[from] tokio::task::JoinError),
}

impl CommitError {
    /// Classifies a store failure; running out of space is its own kind.
    pub(super) fn from_store(error: StoreError) -> Self {
        Self::from_event(EventError::Store(error))
    }

    pub(super) fn from_event(error: EventError) -> Self {
        match &error {
            EventError::Store(store) if store.is_storage_full() => {
                Self::StorageFull { source: error }
            }
            _ => Self::Event(error),
        }
    }

    /// Stable machine-readable kind.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Event(error) => error.kind(),
            Self::StorageFull { .. } => "storage_full",
            Self::RemoteReplayed { .. } => "remote_replayed",
            Self::RemoteMark { .. } | Self::RemoteMarkUnknown => "invalid_event",
            Self::PageExpired { .. } => "page_expired",
            Self::Backpressure { .. } => "backpressure",
            Self::InvalidBound { .. } => "invalid_params",
            Self::Worker(_) => "unavailable",
        }
    }

    /// Recovery guidance independent of display text.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Event(error) => error.hint(),
            Self::StorageFull { .. } => {
                "free space under the store root; nothing was acknowledged or published"
            }
            Self::RemoteReplayed { .. } => {
                "resume the remote line after the committed upstream sequence"
            }
            Self::RemoteMark { .. } | Self::RemoteMarkUnknown => {
                "inspect the quarantined store; the mark is never reset to a default"
            }
            Self::PageExpired { .. } => "pin a fresh snapshot and read it before its expiry",
            Self::Backpressure { .. } => "retry after an in-flight commit returns",
            Self::InvalidBound { .. } => "configure nonzero bounds and a representable expiry",
            Self::Worker(_) => "inspect the operation; its commit may or may not have landed",
        }
    }
}

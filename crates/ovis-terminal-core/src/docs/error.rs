//! Typed document failures and their wire error data.

use ovis_terminal_protocol::domain::documents_retention::DocumentError;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorData, ErrorKind};
use ovis_terminal_protocol::schema::ids::DocName;
use ovis_terminal_protocol::schema::scalar::RecordVersion;
use ovis_terminal_store::StoreError;

/// A document read or write that cannot proceed; none is turned into a default value.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DocError {
    /// The name or value is outside its advertised bounds.
    #[error(transparent)]
    Document(#[from] DocumentError),
    /// The document is not at the version the caller read.
    #[error("document {} is at version {}, not {}", .name.as_str(), .current.get(), .expected.get())]
    Conflict {
        /// Document name.
        name: DocName,
        /// Version the caller read.
        expected: RecordVersion,
        /// Version the document is at.
        current: RecordVersion,
    },
    /// A version or sequence counter cannot advance.
    #[error("the {counter} counter is exhausted")]
    Exhausted {
        /// Which counter.
        counter: &'static str,
    },
    /// The stored record has a shape this build does not read.
    #[error("the stored document record has an unsupported shape")]
    UnsupportedRecord,
    /// The result could not be encoded.
    #[error("the document result could not be encoded")]
    Encode(#[from] serde_json::Error),
    /// The store could not be read.
    #[error("the document store could not be read")]
    Store(#[from] StoreError),
}

impl DocError {
    /// Stable machine token.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Document(error) => error.kind(),
            Self::Conflict { .. } => "conflict",
            Self::Exhausted { .. } => "counter_overflow",
            Self::UnsupportedRecord => "invalid_record",
            Self::Encode(_) => "internal",
            Self::Store(error) => error.kind(),
        }
    }

    /// Recovery guidance.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Document(error) => error.hint(),
            Self::Conflict { .. } => "read doc.get and retry the write with its version",
            Self::Exhausted { .. } => "write the value under a new document name",
            Self::UnsupportedRecord => "open the store with the build that wrote it",
            Self::Encode(_) => "inspect the operation before retrying",
            Self::Store(error) => error.hint(),
        }
    }

    /// The version the document is at, carried by a conflict.
    pub const fn current_version(&self) -> Option<RecordVersion> {
        match self {
            Self::Conflict { current, .. } => Some(*current),
            Self::Document(_)
            | Self::Exhausted { .. }
            | Self::UnsupportedRecord
            | Self::Encode(_)
            | Self::Store(_) => None,
        }
    }

    /// The typed error retained with the failed operation and returned on the wire.
    pub fn error_data(&self) -> ErrorData {
        let retryable = matches!(self, Self::Store(StoreError::Backpressure { .. }));
        BoundaryError::domain(ErrorKind::from(self.kind().to_owned()), retryable)
            .with_hint(self.hint())
            .data()
            .clone()
    }
}

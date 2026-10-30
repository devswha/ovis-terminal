//! Typed failures of the holder socket service and of one core connection.

use std::io;

use thiserror::Error;

use super::error::HolderError;
use crate::protocol::{FrameError, MessageKind};

/// Why the holder dropped a core connection.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConnectionError {
    /// Reading from or writing to the core socket failed.
    #[error("the core socket failed")]
    Io {
        /// Socket failure.
        #[source]
        source: io::Error,
    },
    /// A frame could not be decoded or encoded.
    #[error("a holder frame was rejected")]
    Frame {
        /// Frame failure.
        #[from]
        source: FrameError,
    },
    /// A core request was rejected; the core must re-adopt to resynchronize.
    #[error("a core request was rejected")]
    Holder {
        /// Holder failure.
        #[from]
        source: HolderError,
    },
    /// A message other than `Hello` arrived before adoption.
    #[error("{kind:?} arrived before Hello")]
    HelloRequired {
        /// Message that arrived first.
        kind: MessageKind,
    },
    /// A holder-to-core message or a repeated `Hello` arrived from the core.
    #[error("{kind:?} is not a core-to-holder message here")]
    UnexpectedMessage {
        /// Offending message.
        kind: MessageKind,
    },
    /// The core completed a drain while terminal records remain.
    #[error("drain completion arrived while terminal records remain")]
    DrainIncomplete,
    /// The core read too slowly and its link backlog overflowed.
    #[error("the core link backlog overflowed")]
    LinkOverflow,
    /// The link writer thread panicked.
    #[error("the core link writer panicked")]
    WriterPanicked,
}

impl ConnectionError {
    /// Returns the stable machine-readable error kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Io { .. } => "connection_io",
            Self::Frame { source } => source.kind(),
            Self::Holder { source } => source.kind(),
            Self::HelloRequired { .. } => "hello_required",
            Self::UnexpectedMessage { .. } => "unexpected_message",
            Self::DrainIncomplete => "drain_incomplete",
            Self::LinkOverflow => "link_overflow",
            Self::WriterPanicked => "writer_panicked",
        }
    }

    /// Returns a static recovery hint.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Io { .. } => "Reconnect and re-adopt the holder.",
            Self::Frame { source } => source.hint(),
            Self::Holder { source } => source.hint(),
            Self::HelloRequired { .. } => "Send Hello first on every connection.",
            Self::UnexpectedMessage { .. } => "Send only core-to-holder messages after one Hello.",
            Self::DrainIncomplete => "Forget every exited terminal before completing a drain.",
            Self::LinkOverflow => "Read holder messages promptly, then re-adopt.",
            Self::WriterPanicked => "Report the holder defect and re-adopt.",
        }
    }
}

/// The holder socket service could not start, stop or finish cleanly.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ServeError {
    /// A socket or thread operation failed.
    #[error("the holder service socket or thread failed")]
    Io {
        /// Underlying failure.
        #[source]
        source: io::Error,
    },
    /// The listener has no filesystem path to wake it with.
    #[error("the holder listener is not bound to a filesystem path")]
    UnnamedSocket,
    /// The accept thread panicked.
    #[error("the holder accept thread panicked")]
    ServicePanicked,
    /// Holder shutdown failed after the service stopped.
    #[error("holder shutdown failed")]
    Shutdown {
        /// Shutdown failure.
        #[from]
        source: HolderError,
    },
}

impl ServeError {
    /// Returns the stable machine-readable error kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Io { .. } => "service_io",
            Self::UnnamedSocket => "unnamed_socket",
            Self::ServicePanicked => "service_panicked",
            Self::Shutdown { source } => source.kind(),
        }
    }

    /// Returns a static recovery hint.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Io { .. } => "Check the socket path and process limits.",
            Self::UnnamedSocket => "Bind the holder listener to a filesystem path.",
            Self::ServicePanicked => "Report the holder defect.",
            Self::Shutdown { source } => source.hint(),
        }
    }
}

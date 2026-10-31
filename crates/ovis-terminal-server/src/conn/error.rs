//! Why a connection was closed by the server, and why a hello was refused.

use std::io;

use ovis_terminal_authz::AuthzError;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::schema::ParamsError;
use ovis_terminal_protocol::schema::scalar::ProtocolGeneration;
use ovis_terminal_protocol::wire::FrameError;
use thiserror::Error;

/// A failure that ends one connection; the peer receives `$/closing` when writable.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConnectionError {
    /// The byte stream violates the framing rules.
    #[error("the byte stream is malformed")]
    Frame(#[from] FrameError),
    /// A frame names a channel or kind no stream on this connection owns.
    #[error("frame for channel {channel} has no owner on this connection")]
    UnownedChannel {
        /// The channel named by the frame.
        channel: u32,
    },
    /// A framed control record is not UTF-8.
    #[error("a control record is not UTF-8")]
    InvalidUtf8(#[source] std::str::Utf8Error),
    /// The decoder produced a record kind this server does not handle.
    #[error("the decoder produced an unsupported record")]
    UnsupportedRecord,
    /// A record could not be parsed far enough to recover its request id.
    #[error("a record is not a correlatable JSON-RPC request")]
    Uncorrelatable(#[source] BoundaryError),
    /// The endpoint refused the peer before it could authenticate.
    #[error("the peer was refused")]
    Refused(#[source] HelloError),
    /// Request settlement lost track of a request id.
    #[error("request correlation failed")]
    Correlation(#[source] BoundaryError),
    /// The peer let more encoded bytes queue up than the connection's bound allows.
    #[error("{queued} unwritten bytes were queued against a bound of {limit}")]
    Backpressure {
        /// The connection's unwritten-byte bound.
        limit: u64,
        /// Bytes queued and not yet written when the bound was hit.
        queued: u64,
    },
    /// Reading or writing the transport failed.
    #[error("the transport failed")]
    Io(#[from] io::Error),
    /// An outbound record could not be encoded.
    #[error("an outbound record could not be encoded")]
    Encode(#[from] serde_json::Error),
}

impl ConnectionError {
    /// The stable machine-readable kind, also sent as the `$/closing` reason.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Frame(error) => error.kind(),
            Self::UnownedChannel { .. } => "unowned_channel",
            Self::InvalidUtf8(_) => "invalid_control_utf8",
            Self::UnsupportedRecord => "unsupported_record",
            Self::Uncorrelatable(error) => match error.error_kind() {
                ErrorKind::ParseError => "parse_error",
                _ => "invalid_request",
            },
            Self::Refused(error) => error.kind(),
            Self::Correlation(_) | Self::Encode(_) => "internal",
            Self::Backpressure { .. } => "backpressure",
            Self::Io(_) => "connection_io",
        }
    }

    /// Recovery guidance for the kind.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Frame(error) => error.hint(),
            Self::UnownedChannel { .. } => "send frames only on channel 0 or an opened stream",
            Self::InvalidUtf8(_) => "send UTF-8 JSON-RPC on channel 0",
            Self::UnsupportedRecord => "use a protocol generation this server serves",
            Self::Uncorrelatable(_) => "send JSON-RPC 2.0 requests with a string id",
            Self::Refused(error) => error.hint(),
            Self::Correlation(_) | Self::Encode(_) => "reconnect and report the runtime log",
            Self::Backpressure { .. } => {
                "read replies and notifications as they arrive, then reconnect"
            }
            Self::Io(_) => "reconnect",
        }
    }
}

/// Why a request was refused at the hello boundary.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HelloError {
    /// A method other than `hello` arrived before a successful hello.
    #[error("the connection has not completed hello")]
    HelloRequired,
    /// `hello` arrived on an established connection.
    #[error("this connection already completed hello")]
    AlreadyEstablished,
    /// The hello parameters violate their closed schema.
    #[error("hello parameters are malformed")]
    Params(#[source] ParamsError),
    /// The client speaks a generation this runtime does not serve.
    #[error("generation {} is not served; this runtime speaks {}", .requested.get(), .served.get())]
    UnsupportedGeneration {
        /// The generation the client asked for.
        requested: ProtocolGeneration,
        /// The generation this runtime serves.
        served: ProtocolGeneration,
    },
    /// The endpoint check refused the peer.
    #[error("the endpoint refused this peer")]
    Admission(#[source] AuthzError),
    /// The credential did not authenticate.
    #[error("the hello credential was refused")]
    Credential(#[source] AuthzError),
    /// The server clock cannot date the authentication.
    #[error("the server clock is outside the representable range")]
    Clock,
}

impl HelloError {
    /// The stable machine-readable kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::HelloRequired => "unauthenticated",
            Self::AlreadyEstablished => "invalid_request",
            Self::Params(_) => "invalid_params",
            Self::UnsupportedGeneration { .. } => "unsupported_generation",
            Self::Admission(error) | Self::Credential(error) => error.kind(),
            Self::Clock => "internal",
        }
    }

    /// Recovery guidance for the kind.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::HelloRequired => "send hello first and wait for its result",
            Self::AlreadyEstablished => "open a new connection to negotiate again",
            Self::Params(_) => "correct the field at field_path",
            Self::UnsupportedGeneration { .. } => {
                "reconnect with a client built for this runtime's protocol generation"
            }
            Self::Admission(error) | Self::Credential(error) => error.hint(),
            Self::Clock => "correct the server clock",
        }
    }

    /// The JSON-RPC error carried to the peer.
    #[must_use]
    pub fn to_boundary(&self) -> BoundaryError {
        match self {
            Self::Params(error) => BoundaryError::invalid_params(Some(error.field_path())),
            Self::AlreadyEstablished => BoundaryError::invalid_request(Some("method")),
            Self::HelloRequired
            | Self::UnsupportedGeneration { .. }
            | Self::Admission(_)
            | Self::Credential(_)
            | Self::Clock => BoundaryError::domain(ErrorKind::from(self.kind().to_owned()), false)
                .with_hint(self.hint()),
        }
    }
}

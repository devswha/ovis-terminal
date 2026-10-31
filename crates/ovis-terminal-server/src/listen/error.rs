//! The endpoint boundary error: resolving, binding, accepting and probing.

use std::io;
use std::path::PathBuf;

use ovis_terminal_authz::AuthzError;
use thiserror::Error;

/// Why an endpoint could not be described, served or probed.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EndpointError {
    /// The session name is not 1 to 32 letters, digits, `-` or `_`.
    #[error("the session name is not 1 to 32 letters, digits, '-' or '_'")]
    InvalidSession,
    /// The override is not an absolute socket path or a local pipe name.
    #[error("OVIS_TERMINAL_SOCKET is not an absolute socket path or a local pipe name")]
    InvalidOverride,
    /// No absolute runtime, state or home directory is set.
    #[error("none of XDG_RUNTIME_DIR, XDG_STATE_HOME or HOME names an absolute directory")]
    NoRuntimeDirectory,
    /// The Windows account name cannot name a pipe.
    #[error("USERNAME is unset or cannot be part of a pipe name")]
    InvalidUser,
    /// The socket path exceeds what every supported Unix accepts.
    #[error("the socket path is {length} bytes; at most {limit} are portable")]
    SocketPathTooLong {
        /// Path length in bytes.
        length: usize,
        /// Portable limit.
        limit: usize,
    },
    /// A live server already answers on the socket path.
    #[error("{path} is already served by a live process")]
    AlreadyServing {
        /// The contested socket path.
        path: PathBuf,
    },
    /// The path exists and is not a socket, so it is never replaced.
    #[error("{path} exists and is not a socket")]
    NotASocket {
        /// The occupied path.
        path: PathBuf,
    },
    /// The socket or its directory is reachable by another account.
    #[error("{path} is not private to its owner")]
    Insecure {
        /// The exposed path.
        path: PathBuf,
        /// The endpoint check that refused it.
        #[source]
        source: AuthzError,
    },
    /// The peer disconnected before the operating system could describe it.
    #[error("the peer closed before its credential could be read")]
    PeerClosed {
        /// The failed peer inspection.
        #[source]
        source: io::Error,
    },
    /// An operating-system call on the endpoint failed.
    #[error("{operation} failed on {path}")]
    Io {
        /// What was attempted.
        operation: &'static str,
        /// The endpoint path.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },
    /// The handler names a method outside the published catalog.
    #[error("the handler's method list does not match the published catalog")]
    Methods(#[source] ovis_terminal_protocol::schema::SchemaError),
    /// The server identity could not be turned into a hello result.
    #[error("the server identity does not form a valid hello result")]
    Identity(#[source] serde_json::Error),
    /// A live server refused the probe's hello.
    #[error("the server refused hello")]
    HelloRejected {
        /// The typed error the server returned.
        error: Box<ovis_terminal_protocol::rpc::RpcErrorObject>,
    },
    /// The server answered with something other than a JSON-RPC reply.
    #[error("the server reply is not a JSON-RPC reply")]
    MalformedReply(#[source] ovis_terminal_protocol::errors::BoundaryError),
    /// An advertised limit does not fit this platform's word size.
    #[error("the {limit} limit does not fit this platform")]
    Unrepresentable {
        /// The limit's field name.
        limit: &'static str,
    },
    /// The server task ended by panicking or being cancelled.
    #[error("the server task did not finish normally")]
    ServerTask(#[source] tokio::task::JoinError),
}

impl EndpointError {
    /// The stable machine-readable kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::InvalidSession => "invalid_session",
            Self::InvalidOverride => "invalid_endpoint_override",
            Self::NoRuntimeDirectory => "no_runtime_directory",
            Self::InvalidUser => "invalid_user",
            Self::SocketPathTooLong { .. } => "socket_path_too_long",
            Self::AlreadyServing { .. } => "already_running",
            Self::NotASocket { .. } => "not_a_socket",
            Self::Insecure { .. } => "endpoint_insecure",
            Self::PeerClosed { .. } => "peer_closed",
            Self::Io { .. } => "endpoint_io",
            Self::Methods(_) => "uncatalogued_method",
            Self::Identity(_) => "invalid_identity",
            Self::HelloRejected { .. } => "hello_rejected",
            Self::MalformedReply(_) => "invalid_response",
            Self::Unrepresentable { .. } | Self::ServerTask(_) => "internal",
        }
    }

    /// Recovery guidance for the kind.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::InvalidSession => "set OVIS_TERMINAL_SESSION to letters, digits, '-' or '_'",
            Self::InvalidOverride => {
                "set OVIS_TERMINAL_SOCKET to an absolute path or a \\\\.\\pipe\\ name"
            }
            Self::NoRuntimeDirectory => "set XDG_RUNTIME_DIR or OVIS_TERMINAL_SOCKET",
            Self::InvalidUser => "set USERNAME or OVIS_TERMINAL_SOCKET",
            Self::SocketPathTooLong { .. } => {
                "use a shorter session name or OVIS_TERMINAL_SOCKET path"
            }
            Self::AlreadyServing { .. } => "connect to the running server or stop it first",
            Self::NotASocket { .. } => "move the file away or choose another socket path",
            Self::Insecure { .. } => "restrict the socket directory to its owner (mode 0700)",
            Self::PeerClosed { .. } => "reconnect; the connection was refused before hello",
            Self::Io { .. } => "check the path, its permissions and free descriptors",
            Self::Methods(_) => "register only methods published in the OpenRPC catalog",
            Self::Identity(_) => "report the runtime build; its identity is malformed",
            Self::HelloRejected { .. } => "act on error.data.kind; the server is running",
            Self::MalformedReply(_) => "check that the path belongs to an ovis-terminal server",
            Self::Unrepresentable { .. } => "run a build for a 32- or 64-bit platform",
            Self::ServerTask(_) => "inspect the runtime log and restart the server",
        }
    }
}

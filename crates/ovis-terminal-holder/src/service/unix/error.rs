//! The service boundary error: every way managing a holder service can fail.

use std::io;
use std::path::PathBuf;

use ovis_terminal_protocol::domain::unix_service::{
    BootProof, DiscoveryError, Prerequisite, StaleReason,
};
use thiserror::Error;

use crate::process::StartTokenError;
use crate::unix::{CoreConnectionError, ServeError};

/// Why a service command failed; nothing is reported as success that did not happen.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ServiceError {
    /// The session name is not 1 to 32 ASCII letters, digits, `-` or `_`.
    #[error("the session name is not a valid service session")]
    InvalidSession,
    /// None of the runtime, state or home directories is set to an absolute path.
    #[error("no absolute runtime, state or home directory is set")]
    NoRuntimeDirectory,
    /// The socket path does not fit a Unix socket address.
    #[error("the holder socket path is {length} bytes; the limit is {limit}")]
    SocketPathTooLong {
        /// Path length in bytes.
        length: usize,
        /// Longest accepted path.
        limit: usize,
    },
    /// A path cannot be written into a service definition or record.
    #[error("{path:?} cannot be named in a service definition")]
    UnsupportedPath {
        /// The path refused.
        path: PathBuf,
    },
    /// A filesystem or socket operation failed.
    #[error("{operation} failed")]
    Io {
        /// What was attempted.
        operation: &'static str,
        /// The failure.
        #[source]
        source: io::Error,
    },
    /// The stored discovery record cannot be used.
    #[error("the discovery record cannot be used")]
    Record(#[source] DiscoveryError),
    /// The holder's own start token could not be read.
    #[error("the holder start token could not be read")]
    StartToken(#[source] StartTokenError),
    /// The holder socket service failed.
    #[error("the holder socket service failed")]
    Serve(#[source] ServeError),
    /// Talking to the running holder failed.
    #[error("the holder connection failed")]
    Connection(#[source] CoreConnectionError),
    /// A holder already answers for this service; it is never replaced.
    #[error("a holder already runs for this service")]
    AlreadyRunning {
        /// The live holder's boot proof, when its record named one.
        holder: Option<Box<BootProof>>,
    },
    /// No holder runs for this service.
    #[error("no holder runs for this service")]
    NotRunning,
    /// The record names a holder that is gone.
    #[error("the service endpoint is stale ({reason:?})")]
    Stale {
        /// Why it is stale.
        reason: StaleReason,
    },
    /// The holder lives but its endpoint does not answer.
    #[error("the holder does not answer on its endpoint")]
    Unreachable,
    /// The answering holder is not the incarnation the record names.
    #[error("the answering holder is not the recorded incarnation")]
    IdentityMismatch,
    /// The service manager cannot be used on this host or session.
    #[error("the service manager is unavailable ({missing:?})")]
    ManagerUnavailable {
        /// The prerequisite that is missing.
        missing: Prerequisite,
    },
    /// The service manager is not available on this operating system.
    #[error("this service manager is not supported on this operating system")]
    Unsupported,
    /// A manager call did not finish within its bound; its effect is unknown.
    #[error("{program} did not finish within {bound_ms} ms")]
    ManagerTimeout {
        /// Program called.
        program: &'static str,
        /// Bound waited.
        bound_ms: u64,
    },
    /// A manager call exited unsuccessfully.
    #[error("{program} exited with {code:?}: {stderr}")]
    ManagerFailed {
        /// Program called.
        program: &'static str,
        /// Exit code, when it exited normally.
        code: Option<i32>,
        /// Its trimmed diagnostics.
        stderr: String,
    },
    /// A definition exists at the service path that this runtime did not write.
    #[error("{path:?} is not a registration this runtime wrote")]
    ForeignRegistration {
        /// The definition path.
        path: PathBuf,
    },
    /// No registration exists for the service.
    #[error("no registration exists for this service")]
    NotRegistered,
    /// The launched holder did not report ready within its bound.
    #[error("the holder did not become ready within {bound_ms} ms")]
    NotReady {
        /// Bound waited.
        bound_ms: u64,
    },
}

impl ServiceError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::InvalidSession => "invalid_session",
            Self::NoRuntimeDirectory => "no_runtime_directory",
            Self::SocketPathTooLong { .. } => "socket_path_too_long",
            Self::UnsupportedPath { .. } => "unsupported_path",
            Self::Io { .. } => "service_io",
            Self::Record(source) => source.kind(),
            Self::StartToken(source) => source.kind(),
            Self::Serve(source) => source.kind(),
            Self::Connection(source) => source.kind(),
            Self::AlreadyRunning { .. } => "already_running",
            Self::NotRunning => "already_stopped",
            Self::Stale { .. } => "stale_endpoint",
            Self::Unreachable => "unavailable",
            Self::IdentityMismatch => "identity_mismatch",
            Self::ManagerUnavailable { .. } => "service_manager_unavailable",
            Self::Unsupported => "service_unsupported",
            Self::ManagerTimeout { .. } => "manager_timeout",
            Self::ManagerFailed { .. } => "manager_failed",
            Self::ForeignRegistration { .. } => "foreign_registration",
            Self::NotRegistered => "not_registered",
            Self::NotReady { .. } => "holder_not_ready",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::InvalidSession => "use 1 to 32 ASCII letters, digits, '-' or '_'",
            Self::NoRuntimeDirectory => {
                "set XDG_RUNTIME_DIR, XDG_STATE_HOME or HOME to an absolute path"
            }
            Self::SocketPathTooLong { .. } => "use a shorter runtime directory or session name",
            Self::UnsupportedPath { .. } => "use paths without newlines that are valid UTF-8",
            Self::Io { .. } => "check the directory permissions and free space",
            Self::Record(source) => source.hint(),
            Self::StartToken(source) => source.hint(),
            Self::Serve(source) => source.hint(),
            Self::Connection(source) => source.hint(),
            Self::AlreadyRunning { .. } => "use the running holder, or stop it explicitly first",
            Self::NotRunning => "start the service explicitly; status never starts one",
            Self::Stale { .. } => "start the service again; start replaces a proven-dead endpoint",
            Self::Unreachable => "inspect the holder process; it lives but does not accept",
            Self::IdentityMismatch => "check the status; another process answers at the address",
            Self::ManagerUnavailable { .. } => {
                "meet the listed prerequisite or use --manager detached"
            }
            Self::Unsupported => "use the manager of this operating system",
            Self::ManagerTimeout { .. } => {
                "inspect the service with `service status` before retrying"
            }
            Self::ManagerFailed { .. } => "read the manager diagnostics and the service journal",
            Self::ForeignRegistration { .. } => {
                "remove or rename that definition yourself; it is not ours"
            }
            Self::NotRegistered => "install the service before starting it through a manager",
            Self::NotReady { .. } => "inspect `service status`; the holder may still be starting",
        }
    }
}

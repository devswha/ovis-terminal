//! The Windows service host boundary error.

use std::io;
use std::path::PathBuf;

use ovis_terminal_protocol::domain::unix_service::{BootProof, DiscoveryError, StaleReason};
use ovis_terminal_protocol::domain::windows_service::{ScmState, WindowsServiceRecordError};
use thiserror::Error;

/// Why managing or running a Windows holder service failed.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HostError {
    /// The session name is not 1 to 32 ASCII letters, digits, `-` or `_`.
    #[error("the session name is not a valid service session")]
    InvalidSession,
    /// `LOCALAPPDATA` is unset, so no state directory can be named.
    #[error("no local application data directory is set")]
    NoStateDirectory,
    /// The state directory is not absolute or cannot be written into a command line.
    #[error("{path:?} cannot be the service state directory")]
    UnsupportedPath {
        /// The path refused.
        path: PathBuf,
    },
    /// A pipe name or account is unusable.
    #[error("a service name or account is unusable")]
    Name(#[source] WindowsServiceRecordError),
    /// The stored host record cannot be used.
    #[error("the host record cannot be used")]
    Record(#[source] DiscoveryError),
    /// An operating-system call failed.
    #[error("{operation} failed")]
    Io {
        /// What was attempted.
        operation: &'static str,
        /// The failure.
        #[source]
        source: io::Error,
    },
    /// The service control manager or the pipe refused this account.
    #[error("{operation} was refused for this account")]
    AccessDenied {
        /// What was attempted.
        operation: &'static str,
    },
    /// A holder already serves this session; it is never replaced.
    #[error("a holder already runs for this service")]
    AlreadyRunning {
        /// The live holder's boot proof, when its record named one.
        holder: Option<Box<BootProof>>,
    },
    /// The service is not running.
    #[error("no holder runs for this service")]
    NotRunning,
    /// The record names a holder that is gone.
    #[error("the service endpoint is stale ({reason:?})")]
    Stale {
        /// Why it is stale.
        reason: StaleReason,
    },
    /// The holder lives but its pipe does not answer.
    #[error("the holder does not answer on its pipe")]
    Unreachable,
    /// No service of this label is installed.
    #[error("no service of this label is installed")]
    NotInstalled,
    /// A service of this label is already installed.
    #[error("service {label} is already installed")]
    AlreadyInstalled {
        /// The service name.
        label: String,
    },
    /// Installing needs the account password in `OVIS_TERMINAL_SERVICE_PASSWORD`.
    #[error("the service account password was not provided")]
    PasswordRequired,
    /// The service stopped instead of reaching the requested state.
    #[error("the service stopped with exit code {code}")]
    ServiceFailed {
        /// The service's Win32 exit code.
        code: u32,
    },
    /// A service of this label exists but this runtime did not install it for this owner.
    #[error("service {label} is not a registration this runtime made for this account")]
    ForeignRegistration {
        /// The service name.
        label: String,
    },
    /// The service did not reach the state within the bound; its outcome is unknown.
    #[error("the service did not reach {wanted:?} within {bound_ms} ms")]
    StateTimeout {
        /// State waited for.
        wanted: ScmState,
        /// Bound waited.
        bound_ms: u64,
    },
}

impl HostError {
    /// Wraps the last operating-system error of `operation`, typing access refusals.
    #[must_use]
    pub fn os(operation: &'static str, source: io::Error) -> Self {
        if source.kind() == io::ErrorKind::PermissionDenied {
            Self::AccessDenied { operation }
        } else {
            Self::Io { operation, source }
        }
    }

    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::InvalidSession => "invalid_session",
            Self::NoStateDirectory => "no_runtime_directory",
            Self::UnsupportedPath { .. } => "unsupported_path",
            Self::Name(source) => source.kind(),
            Self::Record(source) => source.kind(),
            Self::Io { .. } => "service_io",
            Self::AccessDenied { .. } => "forbidden",
            Self::AlreadyRunning { .. } => "already_running",
            Self::NotRunning => "already_stopped",
            Self::Stale { .. } => "stale_endpoint",
            Self::Unreachable => "unavailable",
            Self::NotInstalled => "not_registered",
            Self::AlreadyInstalled { .. } => "service_exists",
            Self::ServiceFailed { .. } => "service_failed",
            Self::PasswordRequired => "password_required",
            Self::ForeignRegistration { .. } => "foreign_registration",
            Self::StateTimeout { .. } => "holder_not_ready",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::InvalidSession => "use 1 to 32 ASCII letters, digits, '-' or '_'",
            Self::NoStateDirectory => "set LOCALAPPDATA or pass --state-dir",
            Self::UnsupportedPath { .. } => "pass an absolute state directory without quotes",
            Self::Name(source) => source.hint(),
            Self::Record(source) => source.hint(),
            Self::Io { .. } => "check the service log and the state directory permissions",
            Self::AccessDenied { .. } => {
                "run as the account that owns the service, or elevate to install it"
            }
            Self::AlreadyRunning { .. } => "use the running holder, or stop it explicitly first",
            Self::NotRunning => "start the service explicitly; status never starts one",
            Self::Stale { .. } => "start the service again; start replaces a proven-dead record",
            Self::Unreachable => "inspect the service; its process lives but serves no pipe",
            Self::NotInstalled => "install the service before starting or stopping it",
            Self::AlreadyInstalled { .. } => "use the installed service, or uninstall it first",
            Self::PasswordRequired => {
                "set OVIS_TERMINAL_SERVICE_PASSWORD; it is never read from the command line"
            }
            Self::ServiceFailed { .. } => {
                "read the service's event log entry; the account may lack the logon right"
            }
            Self::ForeignRegistration { .. } => {
                "remove or rename that service yourself; it is not ours"
            }
            Self::StateTimeout { .. } => "inspect `service status` before retrying",
        }
    }
}

//! Typed spawn failures, reported separately from any admitted launch.

use std::error::Error as StdError;
use std::io;
use std::path::PathBuf;

use thiserror::Error;

use crate::process::StartTokenError;
use crate::protocol::{SpawnFailureKind, SpawnFailureReason};

/// Boxed error from the PTY library, preserved as the failure source.
pub type PtySourceError = Box<dyn StdError + Send + Sync + 'static>;

/// A program could not be started on a PTY.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LaunchError {
    /// The argument vector names no program.
    #[error("the argument vector is empty")]
    EmptyArgv,
    /// A grid dimension is zero.
    #[error("terminal grid {cols}x{rows} has a zero dimension")]
    InvalidGrid {
        /// Requested columns.
        cols: u16,
        /// Requested rows.
        rows: u16,
    },
    /// The working directory could not be inspected.
    #[error("working directory {cwd:?} is not accessible")]
    CwdUnavailable {
        /// Requested working directory.
        cwd: PathBuf,
        /// Inspection failure.
        #[source]
        source: io::Error,
    },
    /// The working directory exists but is not a directory.
    #[error("working directory {cwd:?} is not a directory")]
    CwdNotDirectory {
        /// Requested working directory.
        cwd: PathBuf,
    },
    /// No PTY pair or PTY descriptor could be obtained.
    #[error("no pseudo-terminal could be allocated")]
    PtyUnavailable {
        /// PTY allocation failure.
        #[source]
        source: PtySourceError,
    },
    /// The PTY master exposes no file descriptor to duplicate.
    #[error("the pseudo-terminal master has no file descriptor")]
    PtyDescriptorMissing,
    /// The executable path given in argv could not be inspected.
    #[error("executable {path:?} is not accessible")]
    ExecutableUnavailable {
        /// Executable path, resolved against the working directory.
        path: PathBuf,
        /// Inspection failure.
        #[source]
        source: io::Error,
    },
    /// The executable was not found on PATH, is not executable or is a directory.
    #[error("the executable was rejected before it ran")]
    ExecutableRejected {
        /// Resolution or permission failure.
        #[source]
        source: PtySourceError,
    },
    /// The operating system failed to start the program.
    #[error("the program could not be started")]
    ExecFailed {
        /// Spawn failure.
        #[source]
        source: PtySourceError,
    },
    /// The PTY library returned a child handle without exit-signal evidence.
    #[error("the PTY library returned an unsupported child handle")]
    UnsupportedChild {
        /// Failure while stopping and reaping that child, if any.
        cleanup: Option<io::Error>,
    },
    /// The started program's start token could not be read; it was stopped.
    #[error("the started program has no readable start token")]
    StartToken {
        /// Start-token failure.
        #[source]
        source: StartTokenError,
        /// Failure while stopping and reaping the program, if any.
        cleanup: Option<io::Error>,
    },
}

impl LaunchError {
    /// Returns the stable machine-readable error kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::EmptyArgv => "empty_argv",
            Self::InvalidGrid { .. } => "invalid_grid",
            Self::CwdUnavailable { .. } => "cwd_unavailable",
            Self::CwdNotDirectory { .. } => "cwd_not_directory",
            Self::PtyUnavailable { .. } => "pty_unavailable",
            Self::PtyDescriptorMissing => "pty_descriptor_missing",
            Self::ExecutableUnavailable { .. } => "executable_unavailable",
            Self::ExecutableRejected { .. } => "executable_rejected",
            Self::ExecFailed { .. } => "exec_failed",
            Self::UnsupportedChild { .. } => "unsupported_child_handle",
            Self::StartToken { .. } => "start_token_unavailable",
        }
    }

    /// Returns a static recovery hint.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::EmptyArgv => "Pass the program as the first argv element.",
            Self::InvalidGrid { .. } => "Request at least one column and one row.",
            Self::CwdUnavailable { .. } | Self::CwdNotDirectory { .. } => {
                "Pass an existing directory as the working directory."
            }
            Self::PtyUnavailable { .. } => "Close unused terminals or raise the PTY limit.",
            Self::PtyDescriptorMissing => "Report the platform; its PTY master has no descriptor.",
            Self::ExecutableUnavailable { .. } | Self::ExecutableRejected { .. } => {
                "Pass an existing executable path, or a PATH entry in the explicit environment."
            }
            Self::ExecFailed { .. } | Self::UnsupportedChild { .. } => {
                "Retry the launch; the holder kept no record of it."
            }
            Self::StartToken { .. } => "Check that the holder can read the process table.",
        }
    }

    /// Returns the wire failure kind reported in `SpawnFailed`.
    #[must_use]
    pub const fn failure_kind(&self) -> SpawnFailureKind {
        match self {
            Self::EmptyArgv
            | Self::InvalidGrid { .. }
            | Self::CwdUnavailable { .. }
            | Self::CwdNotDirectory { .. }
            | Self::ExecutableUnavailable { .. }
            | Self::ExecutableRejected { .. } => SpawnFailureKind::InvalidRequest,
            Self::PtyUnavailable { .. } => SpawnFailureKind::LimitExceeded,
            Self::PtyDescriptorMissing
            | Self::ExecFailed { .. }
            | Self::UnsupportedChild { .. }
            | Self::StartToken { .. } => SpawnFailureKind::Unavailable,
        }
    }

    /// Returns the typed cause reported in `SpawnFailed`.
    #[must_use]
    pub const fn reason(&self) -> SpawnFailureReason {
        match self {
            Self::EmptyArgv => SpawnFailureReason::EmptyArgv,
            Self::InvalidGrid { .. } => SpawnFailureReason::InvalidGrid,
            Self::CwdUnavailable { .. } => SpawnFailureReason::CwdUnavailable,
            Self::CwdNotDirectory { .. } => SpawnFailureReason::CwdNotDirectory,
            Self::PtyUnavailable { .. } => SpawnFailureReason::PtyUnavailable,
            Self::PtyDescriptorMissing => SpawnFailureReason::PtyDescriptorMissing,
            Self::ExecutableUnavailable { .. } => SpawnFailureReason::ExecutableUnavailable,
            Self::ExecutableRejected { .. } => SpawnFailureReason::ExecutableRejected,
            Self::ExecFailed { .. } => SpawnFailureReason::ExecFailed,
            Self::UnsupportedChild { .. } => SpawnFailureReason::UnsupportedChildHandle,
            Self::StartToken { .. } => SpawnFailureReason::StartTokenUnavailable,
        }
    }

    /// Returns the operating-system error number behind the failure, if any.
    #[must_use]
    pub fn os_error(&self) -> Option<i32> {
        match self {
            Self::CwdUnavailable { source, .. } | Self::ExecutableUnavailable { source, .. } => {
                source.raw_os_error()
            }
            Self::PtyUnavailable { source }
            | Self::ExecutableRejected { source }
            | Self::ExecFailed { source } => source
                .downcast_ref::<io::Error>()
                .and_then(io::Error::raw_os_error),
            Self::StartToken {
                source: StartTokenError::Unreadable { source },
                ..
            } => source.raw_os_error(),
            Self::EmptyArgv
            | Self::InvalidGrid { .. }
            | Self::CwdNotDirectory { .. }
            | Self::PtyDescriptorMissing
            | Self::UnsupportedChild { .. }
            | Self::StartToken { .. } => None,
        }
    }

    /// Classifies a PTY-library spawn failure by its underlying I/O error.
    pub(super) fn from_spawn(source: PtySourceError) -> Self {
        let rejected = source.downcast_ref::<io::Error>().is_none_or(|error| {
            matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
            )
        });
        if rejected {
            Self::ExecutableRejected { source }
        } else {
            Self::ExecFailed { source }
        }
    }
}

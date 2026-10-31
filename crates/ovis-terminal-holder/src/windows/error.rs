//! Typed failures of pseudoconsole terminals.

use std::error::Error as StdError;
use std::io;
use std::path::PathBuf;

use ovis_terminal_protocol::domain::conpty::ConptyLaunchError;
use thiserror::Error;

use super::model::OutputError;

type LibraryError = Box<dyn StdError + Send + Sync + 'static>;

/// A pseudoconsole terminal request failed; a failed spawn kept no process.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConptyError {
    /// The launch was refused before any pseudoconsole was allocated.
    #[error("the launch cannot start on a pseudoconsole")]
    Launch(#[from] ConptyLaunchError),
    /// The working directory is not an accessible directory.
    #[error("working directory {cwd:?} is not an accessible directory")]
    CwdUnavailable {
        /// Requested directory.
        cwd: PathBuf,
        /// Inspection failure.
        #[source]
        source: io::Error,
    },
    /// No pseudoconsole or pipe pair could be created.
    #[error("no pseudoconsole could be created")]
    PseudoconsoleUnavailable {
        /// PTY library failure.
        #[source]
        source: LibraryError,
    },
    /// The operating system did not start the program.
    #[error("the program could not be started")]
    ProgramUnavailable {
        /// PTY library failure, with the OS error.
        #[source]
        source: LibraryError,
    },
    /// The started program could not be placed in a job or identified; it was stopped.
    #[error("the started program could not be placed under holder ownership")]
    Ownership {
        /// Job or process-handle failure.
        #[source]
        source: io::Error,
        /// Failure while stopping the program, if any.
        cleanup: Option<io::Error>,
    },
    /// A worker thread could not be started; the program was stopped.
    #[error("a terminal worker thread could not be started")]
    WorkerUnavailable {
        /// Thread creation failure.
        #[source]
        source: io::Error,
    },
    /// The program has exited; only its output and exit remain.
    #[error("the program has exited")]
    ProcessExited,
    /// The pseudoconsole rejected the new grid.
    #[error("the pseudoconsole rejected the new grid")]
    Resize {
        /// PTY library failure.
        #[source]
        source: LibraryError,
    },
    /// The job could not be terminated.
    #[error("the program's job could not be terminated")]
    Kill {
        /// Termination failure.
        #[source]
        source: io::Error,
    },
    /// The output log refused the read.
    #[error("the output could not be read at that offset")]
    Output(#[from] OutputError),
    /// Worker threads panicked before they could be joined.
    #[error("{count} terminal worker thread(s) panicked")]
    WorkerPanicked {
        /// Panicked workers.
        count: usize,
    },
}

impl ConptyError {
    pub(super) fn pseudoconsole(source: impl Into<LibraryError>) -> Self {
        Self::PseudoconsoleUnavailable {
            source: source.into(),
        }
    }

    pub(super) fn program(source: impl Into<LibraryError>) -> Self {
        Self::ProgramUnavailable {
            source: source.into(),
        }
    }

    pub(super) fn resize(source: impl Into<LibraryError>) -> Self {
        Self::Resize {
            source: source.into(),
        }
    }

    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Launch(source) => source.kind(),
            Self::CwdUnavailable { .. }
            | Self::PseudoconsoleUnavailable { .. }
            | Self::ProgramUnavailable { .. }
            | Self::Ownership { .. } => "spawn_failed",
            Self::WorkerUnavailable { .. } => "worker_unavailable",
            Self::ProcessExited => "process_exited",
            Self::Resize { .. } => "resize_failed",
            Self::Kill { .. } => "signal_failed",
            Self::Output(source) => source.kind(),
            Self::WorkerPanicked { .. } => "worker_panicked",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Launch(source) => source.hint(),
            Self::CwdUnavailable { .. } => "pass an existing directory as the working directory",
            Self::PseudoconsoleUnavailable { .. } => {
                "check that this Windows build supports ConPTY (1809 or later)"
            }
            Self::ProgramUnavailable { .. } => {
                "pass an existing executable path, or a PATH entry in the explicit environment"
            }
            Self::Ownership { .. } => "check that the holder may create and assign job objects",
            Self::WorkerUnavailable { .. } => "free thread or memory resources and relaunch",
            Self::ProcessExited => "read the exit; the program can no longer be driven",
            Self::Resize { .. } => "retry with a grid the pseudoconsole accepts",
            Self::Kill { .. } => "read the exit; the program may already have ended",
            Self::Output(source) => source.hint(),
            Self::WorkerPanicked { .. } => "report the holder defect",
        }
    }
}

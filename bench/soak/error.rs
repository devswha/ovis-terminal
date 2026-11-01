//! The one boundary error of the soak harness and its control process.

use std::io;

use ovis_terminal_core::recovery::RecoveryError;
use ovis_terminal_holder::unix::CoreConnectionError;
use ovis_terminal_probes::platform::ProbeError;
use thiserror::Error;

/// A soak run, control process or emitter failure; never turned into a pass.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SoakError {
    /// The arguments describe no runnable soak.
    #[error("invalid arguments: {reason}")]
    Usage {
        /// What is wrong.
        reason: &'static str,
    },
    /// The run would share state with the host.
    #[error("the run must use a fresh private root")]
    NotIsolated,
    /// The `--inject` value names no known fixture.
    #[error("the injected fixture is not one this harness knows")]
    Inject {
        /// Parse failure.
        #[source]
        source: serde_json::Error,
    },
    /// The binary under test could not be read or started.
    #[error("the binary under test could not be read or run")]
    Binary {
        /// I/O failure.
        #[source]
        source: io::Error,
    },
    /// A file, socket or process operation failed.
    #[error("{operation} failed")]
    Io {
        /// What was being done.
        operation: &'static str,
        /// I/O failure.
        #[source]
        source: io::Error,
    },
    /// A JSON record could not be encoded or decoded.
    #[error("a JSON record could not be encoded or decoded")]
    Json {
        /// Codec failure.
        #[from]
        source: serde_json::Error,
    },
    /// A holder of the binary under test exited or stayed silent instead of serving.
    #[error("holder session {session} did not announce readiness: {stderr}")]
    HolderNotReady {
        /// Session name.
        session: String,
        /// What the holder wrote on stderr.
        stderr: String,
    },
    /// A bounded wait ended before the awaited fact arrived.
    #[error("no {waiting_for} within {bound_ms} ms")]
    Timeout {
        /// The awaited fact.
        waiting_for: &'static str,
        /// The bound.
        bound_ms: u64,
    },
    /// A child exited before it printed its first line.
    #[error("the child exited before its {waiting_for}")]
    ChildExited {
        /// The awaited line.
        waiting_for: &'static str,
    },
    /// The control process or its socket went away while a fact was awaited.
    #[error("the control process went away while awaiting {waiting_for}")]
    ControlGone {
        /// The awaited fact.
        waiting_for: &'static str,
    },
    /// A holder link failed.
    #[error("a holder link failed")]
    Holder {
        /// Link failure.
        #[source]
        source: Box<CoreConnectionError>,
    },
    /// A holder answered with a message the protocol does not allow here.
    #[error("the holder sent something other than {expected}")]
    Protocol {
        /// What was expected.
        expected: &'static str,
    },
    /// Recovery could not run.
    #[error("recovery failed")]
    Recovery {
        /// Recovery failure.
        #[source]
        source: Box<RecoveryError>,
    },
    /// A process or memory probe failed.
    #[error("a process probe failed")]
    Probe {
        /// Probe failure.
        #[from]
        source: ProbeError,
    },
    /// The run finished and at least one verdict rule failed.
    #[error("{violations} verdict rule(s) failed")]
    Failed {
        /// Number of failed rules.
        violations: usize,
    },
}

impl SoakError {
    /// Stable machine-readable kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Usage { .. } => "usage",
            Self::NotIsolated => "not_isolated",
            Self::Inject { .. } => "invalid_inject",
            Self::Binary { .. } => "binary_unavailable",
            Self::Io { .. } => "io",
            Self::Json { .. } => "json",
            Self::HolderNotReady { .. } => "holder_not_ready",
            Self::Timeout { .. } => "timeout",
            Self::ChildExited { .. } => "child_exited",
            Self::ControlGone { .. } => "control_gone",
            Self::Holder { .. } => "holder_link",
            Self::Protocol { .. } => "holder_protocol",
            Self::Recovery { .. } => "recovery_failed",
            Self::Probe { .. } => "probe_failed",
            Self::Failed { .. } => "soak_failed",
        }
    }

    /// What to do about it.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Usage { .. } => "see `soak --help`",
            Self::NotIsolated => "pass --isolated; the soak never uses the host's session",
            Self::Inject { .. } => {
                "pass --inject '{\"fixture\":\"holder-tail-gap-and-lost-receipt\"}'"
            }
            Self::Binary { .. } => "build it with cargo build --release -p ovis-terminal-cli",
            Self::Io { .. } | Self::Json { .. } => "inspect the evidence directory and free space",
            Self::HolderNotReady { .. } => "run the binary's `service run` by hand to see why",
            Self::Timeout { .. } | Self::ControlGone { .. } | Self::ChildExited { .. } => {
                "read the control process stderr companion beside the evidence"
            }
            Self::Holder { .. } | Self::Protocol { .. } | Self::Recovery { .. } => {
                "the holder or recovery contract was broken; keep the evidence and report it"
            }
            Self::Probe { .. } => "check that the process table is readable on this host",
            Self::Failed { .. } => "read verdict.violations in the evidence",
        }
    }

    /// Process exit status: 2 for arguments, 1 otherwise.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Usage { .. } | Self::NotIsolated | Self::Inject { .. } => 2,
            _ => 1,
        }
    }

    pub(super) const fn io(operation: &'static str) -> impl FnOnce(io::Error) -> Self {
        move |source| Self::Io { operation, source }
    }
}

impl From<CoreConnectionError> for SoakError {
    fn from(source: CoreConnectionError) -> Self {
        Self::Holder {
            source: Box::new(source),
        }
    }
}

impl From<RecoveryError> for SoakError {
    fn from(source: RecoveryError) -> Self {
        Self::Recovery {
            source: Box::new(source),
        }
    }
}

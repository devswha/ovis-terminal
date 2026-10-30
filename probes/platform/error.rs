//! The probe boundary error.

use std::io;
use std::time::Duration;

use ovis_terminal_holder::protocol::SpawnFailureKind;
use ovis_terminal_vt::VtError;
use serde::Serialize;
use thiserror::Error;

use super::measure::ByteCount;
use super::process::Pid;

/// An error reported by a pseudo-terminal backend that has no concrete type.
pub(super) type BackendError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Pseudo-terminal operations that can fail after the terminal exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PtyOperation {
    /// Cloning the output reader.
    CloneReader,
    /// Taking the input writer.
    TakeWriter,
    /// Starting a pump or waiter thread.
    StartThread,
    /// Writing input.
    Write,
    /// Changing the grid.
    Resize,
    /// Reading the grid.
    ReadSize,
    /// Signalling the program.
    Kill,
    /// Collecting the exit status.
    Wait,
}

/// Endpoint operations probed for socket and pipe identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EndpointOperation {
    /// Creating the listening endpoint.
    Bind,
    /// Connecting a client.
    Connect,
    /// Accepting a client.
    Accept,
    /// Exchanging bytes.
    Exchange,
    /// Reading the endpoint's filesystem identity.
    Inspect,
}

/// Capabilities a platform may lack.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Capability {
    /// An OS token that tells a process apart from a later one reusing its PID.
    ProcessStartToken,
    /// Resident memory of a process.
    ResidentMemory,
}

/// Every failure a platform probe can report.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProbeError {
    /// The backend could not create a terminal pair.
    #[error("the pseudo-terminal backend could not open a terminal pair")]
    Unavailable {
        /// Backend failure.
        #[source]
        source: BackendError,
    },
    /// The program could not be started inside the terminal.
    #[error("the program could not be started inside the pseudo-terminal")]
    SpawnFailed {
        /// Backend failure.
        #[source]
        source: BackendError,
    },
    /// A backend call without an I/O error type failed.
    #[error("pseudo-terminal backend call {operation:?} failed")]
    Backend {
        /// The failing operation.
        operation: PtyOperation,
        /// Backend failure.
        #[source]
        source: BackendError,
    },
    /// An I/O call on the terminal failed.
    #[error("pseudo-terminal call {operation:?} failed")]
    Io {
        /// The failing operation.
        operation: PtyOperation,
        /// Operating-system failure.
        #[source]
        source: io::Error,
    },
    /// The expected bytes or exit did not arrive in time.
    #[error("nothing expected arrived within {waited:?}")]
    Timeout {
        /// How long the probe waited.
        waited: Duration,
    },
    /// The output stream ended before the expected bytes.
    #[error("the terminal output stream ended before the expected bytes")]
    OutputEnded,
    /// More output arrived without the expected bytes than the probe holds.
    #[error("more than {limit:?} bytes arrived without the expected bytes")]
    OutputLimitExceeded {
        /// The bound on held output.
        limit: ByteCount,
    },
    /// The grid cannot be represented by the VT engine.
    #[error("the VT engine cannot hold this grid")]
    InvalidGrid {
        /// VT engine rejection.
        #[source]
        source: VtError,
    },
    /// The OS process record could not be read.
    #[error("the OS record of process {pid} could not be read")]
    ProcessRecordUnreadable {
        /// The probed process.
        pid: Pid,
        /// Operating-system failure.
        #[source]
        source: io::Error,
    },
    /// The OS process record did not contain the expected field.
    #[error("the OS record of process {pid} lacks a readable {field}")]
    ProcessRecordMalformed {
        /// The probed process.
        pid: Pid,
        /// The missing field.
        field: &'static str,
    },
    /// A probe fixture printed a report the probe cannot parse.
    #[error("the fixture report does not match `{report}`")]
    MalformedReport {
        /// The expected report shape.
        report: &'static str,
    },
    /// This platform has no probe for the capability.
    #[error("no probe exists for {capability:?} on this platform")]
    Unsupported {
        /// The missing capability.
        capability: Capability,
    },
    /// A socket or pipe operation failed.
    #[error("endpoint call {operation:?} failed")]
    Endpoint {
        /// The failing operation.
        operation: EndpointOperation,
        /// Operating-system failure.
        #[source]
        source: io::Error,
    },
}

impl ProbeError {
    /// Stable machine-readable kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "unavailable",
            Self::SpawnFailed { .. } => "spawn_failed",
            Self::Backend { .. } => "backend_failed",
            Self::Io { .. } => "io_failed",
            Self::Timeout { .. } => "timeout",
            Self::OutputEnded => "output_ended",
            Self::OutputLimitExceeded { .. } => "limit_exceeded",
            Self::InvalidGrid { .. } => "invalid_grid",
            Self::ProcessRecordUnreadable { .. } => "process_record_unreadable",
            Self::ProcessRecordMalformed { .. } => "process_record_malformed",
            Self::MalformedReport { .. } => "malformed_report",
            Self::Unsupported { .. } => "unsupported",
            Self::Endpoint { .. } => "endpoint_failed",
        }
    }

    /// Recovery guidance for the owning backend.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => {
                "check pseudo-terminal and descriptor limits; the backend cannot hold terminals here"
            }
            Self::SpawnFailed { .. } => "check the executable path and permissions before retrying",
            Self::Backend { .. } | Self::Io { .. } => {
                "treat the terminal as broken and release it; do not report the operation as done"
            }
            Self::Timeout { .. } => {
                "the program did not answer in time; inspect it or raise the bound"
            }
            Self::OutputEnded => {
                "the program exited or closed the terminal before producing the bytes"
            }
            Self::OutputLimitExceeded { .. } => {
                "the program floods output; consume it or stop the program"
            }
            Self::InvalidGrid { .. } => "use at least two columns and one row",
            Self::ProcessRecordUnreadable { .. } => {
                "the process is gone or its record is not readable by this user"
            }
            Self::ProcessRecordMalformed { .. } => {
                "the OS process record changed format; update the probe before trusting tokens"
            }
            Self::MalformedReport { .. } => {
                "rebuild the fixture from this revision; its report format changed"
            }
            Self::Unsupported { .. } => {
                "report the capability as unsupported; never substitute a guess"
            }
            Self::Endpoint { .. } => {
                "the endpoint is unusable; rebind it and re-check its identity"
            }
        }
    }

    /// The holder-protocol failure kind for errors that prevent a spawn.
    #[must_use]
    pub const fn spawn_failure_kind(&self) -> Option<SpawnFailureKind> {
        match self {
            Self::Unavailable { .. } | Self::SpawnFailed { .. } => {
                Some(SpawnFailureKind::Unavailable)
            }
            Self::Backend { .. }
            | Self::Io { .. }
            | Self::Timeout { .. }
            | Self::OutputEnded
            | Self::OutputLimitExceeded { .. }
            | Self::InvalidGrid { .. }
            | Self::ProcessRecordUnreadable { .. }
            | Self::ProcessRecordMalformed { .. }
            | Self::MalformedReport { .. }
            | Self::Unsupported { .. }
            | Self::Endpoint { .. } => None,
        }
    }
}

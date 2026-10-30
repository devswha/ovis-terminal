//! The harness boundary error.

use ovis_terminal_probes::platform::ProbeError;
use thiserror::Error;

/// Every failure a harness run can report.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HarnessError {
    /// A terminal or process probe failed.
    #[error("a platform probe failed during the harness run")]
    Probe(#[from] ProbeError),
    /// The profile asks for no terminals or no time.
    #[error("the harness profile measures nothing")]
    EmptyProfile,
    /// A terminal's echo did not contain what was written.
    #[error("terminal {terminal} echoed other bytes than it was sent")]
    EchoMismatch {
        /// Index of the terminal in the run.
        terminal: u16,
    },
}

impl HarnessError {
    /// Stable machine-readable kind; probe failures keep the probe's kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Probe(source) => source.kind(),
            Self::EmptyProfile => "invalid_profile",
            Self::EchoMismatch { .. } => "echo_mismatch",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Probe(source) => source.hint(),
            Self::EmptyProfile => "ask for at least one terminal and a nonzero duration",
            Self::EchoMismatch { .. } => "the terminal altered input; inspect its line discipline",
        }
    }
}

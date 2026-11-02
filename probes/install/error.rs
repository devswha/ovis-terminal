//! Why the install probe could not reach a verdict; a refused installer run is a finding.

use std::path::PathBuf;

/// A probe run that could not evaluate the installer at all.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum InstallProbeError {
    /// The command line is not a runnable invocation of the probe.
    #[error("{detail}")]
    Usage {
        /// Parser or argument message.
        detail: String,
    },
    /// A scratch file or directory could not be read or written.
    #[error("cannot {operation} {path}", path = path.display())]
    Io {
        /// What was being done.
        operation: &'static str,
        /// Path that failed.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The release stage refused to build the local fixture release.
    #[error("release-stage could not stage the fixture release: {message}")]
    Fixture {
        /// The stage's own message.
        message: String,
    },
    /// A program the probe runs could not be started or waited for.
    #[error("cannot run {program}")]
    Spawn {
        /// The program or command line.
        program: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// An awaited event did not arrive within its hang guard.
    #[error("{what} did not happen within {bound_ms} ms")]
    HangGuard {
        /// The awaited event.
        what: &'static str,
        /// The bound.
        bound_ms: u64,
    },
    /// A document the probe reads or writes is not the expected JSON.
    #[error("{what} is not the expected JSON")]
    Json {
        /// The document.
        what: &'static str,
        /// Parser error.
        #[source]
        source: serde_json::Error,
    },
}

impl InstallProbeError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Usage { .. } => "usage",
            Self::Io { .. } => "io",
            Self::Fixture { .. } => "fixture_failed",
            Self::Spawn { .. } => "spawn_failed",
            Self::HangGuard { .. } => "hang_guard",
            Self::Json { .. } => "json",
        }
    }

    /// What an operator does about it.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Usage { .. } => {
                "run install-probe --script ./install.sh --isolated [--case NAME] [--evidence FILE]"
            }
            Self::Io { .. } => "check that the scratch directory is writable and has free space",
            Self::Fixture { .. } => {
                "build the binary under test first; release-stage needs the repository's schema and notices"
            }
            Self::Spawn { .. } => {
                "the probe needs sh, tar, gzip, curl or wget, and sha256sum or shasum"
            }
            Self::HangGuard { .. } => {
                "the installer or fixture server stopped making progress; see stderr"
            }
            Self::Json { .. } => "the installer must print exactly one JSON report on stdout",
        }
    }

    /// Exit status: 2 usage, 3 anything that kept the probe from a verdict.
    #[must_use]
    pub const fn exit(&self) -> u8 {
        match self {
            Self::Usage { .. } => 2,
            Self::Io { .. }
            | Self::Fixture { .. }
            | Self::Spawn { .. }
            | Self::HangGuard { .. }
            | Self::Json { .. } => 3,
        }
    }

    pub(super) fn io(
        operation: &'static str,
        path: &std::path::Path,
    ) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.to_path_buf();
        move |source| Self::Io {
            operation,
            path,
            source,
        }
    }
}

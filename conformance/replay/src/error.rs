//! Why a fixture set could not be loaded or a case could not be replayed.

use std::path::PathBuf;

/// A failure of the replay boundary; assertion outcomes are [`crate::Check`]s instead.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReplayError {
    /// A fixture or pinned source could not be read.
    #[error("cannot {operation} {path:?}")]
    Io {
        /// What was attempted.
        operation: &'static str,
        /// The file.
        path: PathBuf,
        /// The I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// A fixture file is not valid JSON of its closed format.
    #[error("{path:?} is not a valid fixture file")]
    Parse {
        /// The file.
        path: PathBuf,
        /// The decoding failure.
        #[source]
        source: serde_json::Error,
    },
    /// A fixture file declares another format than the one this build reads.
    #[error("{path:?} declares format {found}, expected {expected}")]
    Format {
        /// The file.
        path: PathBuf,
        /// The format this build reads.
        expected: &'static str,
        /// The declared format.
        found: String,
    },
    /// A `$bind:` or `$value:` symbol has no value when the case reaches it.
    #[error("the symbol {symbol} is not bound")]
    Unbound {
        /// The symbol as written.
        symbol: String,
    },
    /// A legacy step names a method the legacy map does not translate.
    #[error("the legacy method {method} has no translation")]
    UnknownLegacy {
        /// The legacy method.
        method: String,
    },
    /// A legacy step's params are not an object.
    #[error("the params of legacy step {step} are not an object")]
    LegacyParams {
        /// The step identity.
        step: String,
    },
    /// The transport failed to deliver a request or a reply.
    #[error("the transport failed during step {step}")]
    Transport {
        /// The step identity.
        step: String,
        /// The transport failure.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl ReplayError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Io { .. } => "fixture_io",
            Self::Parse { .. } => "fixture_invalid",
            Self::Format { .. } => "fixture_format",
            Self::Unbound { .. } => "symbol_unbound",
            Self::UnknownLegacy { .. } | Self::LegacyParams { .. } => "legacy_untranslated",
            Self::Transport { .. } => "transport_failed",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Io { .. } => {
                "run from a checkout that holds the fixture set and its pinned sources"
            }
            Self::Parse { .. } => {
                "repair the fixture file; every key of the closed format is typed"
            }
            Self::Format { .. } => "replay the fixture set with a build of the matching format",
            Self::Unbound { .. } => {
                "capture the value earlier in the case or supply the server binding"
            }
            Self::UnknownLegacy { .. } | Self::LegacyParams { .. } => {
                "add the method to legacy.json or call the published method"
            }
            Self::Transport { .. } => "inspect the transport's own error; the case stopped there",
        }
    }
}

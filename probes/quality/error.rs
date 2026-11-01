//! Why a gate could not reach a verdict at all.

use std::path::PathBuf;

/// A gate that could not evaluate its inputs; a defect it found is a finding instead.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum QualityError {
    /// `--help` or `--version` was requested; the text is the answer, not a failure.
    #[error("{text}")]
    Help {
        /// Rendered help or version text.
        text: String,
    },
    /// The command line is not a runnable invocation of the tool.
    #[error("{detail}")]
    Usage {
        /// Parser message.
        detail: String,
    },
    /// A limit flag asks for more than the binding contract allows.
    #[error("--{flag} {value} exceeds the binding contract limit {limit}")]
    LimitAboveContract {
        /// Flag name without dashes.
        flag: &'static str,
        /// Requested value.
        value: usize,
        /// Contract limit.
        limit: usize,
    },
    /// `--fixture` names no seeded fixture of this tool.
    #[error("the tool ships no fixture named {name}; it ships {}", known.join(", "))]
    UnknownFixture {
        /// Requested fixture name.
        name: String,
        /// Every fixture the tool ships.
        known: &'static [&'static str],
    },
    /// An input file or directory cannot be read or an output cannot be written.
    #[error("cannot access {path}", path = path.display())]
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A source root cannot be listed or one of its files read.
    #[error("cannot list or read the Rust sources")]
    Walk {
        /// Walker error naming the path.
        #[source]
        source: workspace_hygiene::walk::WalkError,
    },
    /// An input JSON document the tool depends on does not parse.
    #[error("{path} is not the JSON this tool reads", path = path.display())]
    Json {
        /// Offending document.
        path: PathBuf,
        /// Parser error.
        #[source]
        source: serde_json::Error,
    },
    /// A ledger or plan lacks the structure every rule relies on.
    #[error("{path} lacks {missing}", path = path.display())]
    Ledger {
        /// Offending document.
        path: PathBuf,
        /// What could not be found.
        missing: String,
    },
}

impl QualityError {
    /// Stable machine token.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Help { .. } => "help",
            Self::Usage { .. } => "usage",
            Self::LimitAboveContract { .. } => "limit_above_contract",
            Self::UnknownFixture { .. } => "unknown_fixture",
            Self::Io { .. } | Self::Walk { .. } => "io",
            Self::Json { .. } => "input_json",
            Self::Ledger { .. } => "ledger_invalid",
        }
    }

    /// Recovery guidance.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Help { .. } => "the requested text is in detail",
            Self::Usage { .. } => "run the tool with --help for its flags",
            Self::LimitAboveContract { .. } => {
                "limits may only be tightened: at most 250 pure LOC and 3 parameters"
            }
            Self::UnknownFixture { .. } => "pick one of the fixture names in the detail",
            Self::Io { .. } | Self::Walk { .. } => "check the path and its permissions",
            Self::Json { .. } => "regenerate the document with quality-matrix --write",
            Self::Ledger { .. } => "pass the binding ledger or plan file, unmodified",
        }
    }

    /// Process exit status: 2 for usage errors, 3 when inputs cannot be evaluated.
    pub const fn exit(&self) -> u8 {
        match self {
            Self::Help { .. } => 0,
            Self::Usage { .. } | Self::LimitAboveContract { .. } | Self::UnknownFixture { .. } => 2,
            Self::Io { .. } | Self::Walk { .. } | Self::Json { .. } | Self::Ledger { .. } => 3,
        }
    }

    /// A clap parse failure: help and version are answers, everything else a usage error.
    pub(super) fn from_clap(error: &clap::Error) -> Self {
        let text = error.render().to_string();
        if error.use_stderr() {
            Self::Usage { detail: text }
        } else {
            Self::Help { text }
        }
    }

    pub(super) fn io(path: &std::path::Path) -> impl FnOnce(std::io::Error) -> Self + '_ {
        move |source| Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

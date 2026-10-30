//! Findings reported by the hygiene checks.

use std::fmt;
use std::path::PathBuf;

use crate::{MAX_PARAMETERS, MAX_PURE_LOC};

/// What a source file violated.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Finding {
    /// The file has more pure lines of code than [`MAX_PURE_LOC`].
    PureLoc {
        /// Pure lines of code counted in the file.
        count: usize,
    },
    /// A function signature has more parameters than [`MAX_PARAMETERS`].
    Parameters {
        /// Name of the offending function.
        function: String,
        /// Parameters counted, receiver included.
        count: usize,
    },
    /// The file is not valid Rust, so no other rule could be evaluated.
    Syntax {
        /// Parser message.
        message: String,
    },
}

impl Finding {
    /// Stable rule identifier printed in reports.
    pub const fn rule(&self) -> &'static str {
        match self {
            Self::PureLoc { .. } => "max-pure-loc",
            Self::Parameters { .. } => "max-parameters",
            Self::Syntax { .. } => "syntax",
        }
    }
}

/// One finding at a source location.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Violation {
    /// File the finding belongs to.
    pub path: PathBuf,
    /// One-based line of the finding.
    pub line: usize,
    /// What was violated.
    pub finding: Finding,
}

impl Violation {
    /// Creates a violation at `path:line`.
    pub const fn new(path: PathBuf, line: usize, finding: Finding) -> Self {
        Self {
            path,
            line,
            finding,
        }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let location = self.path.display();
        let rule = self.finding.rule();
        match &self.finding {
            Finding::PureLoc { count } => write!(
                f,
                "{location}:{}: rule={rule} count={count} limit={MAX_PURE_LOC}",
                self.line
            ),
            Finding::Parameters { function, count } => write!(
                f,
                "{location}:{}: rule={rule} fn={function} count={count} limit={MAX_PARAMETERS}",
                self.line
            ),
            Finding::Syntax { message } => {
                write!(f, "{location}:{}: rule={rule} message={message}", self.line)
            }
        }
    }
}

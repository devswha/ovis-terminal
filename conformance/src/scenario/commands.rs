//! The commands step: planned runs of the binary, each named by its command line.

use serde::Deserialize;

use super::model::Run;

/// Planned runs of the binary; a run's command line is `ovis-terminal <args>`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Commands {
    pub(crate) planned: Vec<Run>,
}

impl Run {
    /// The command line `--inject` names this run by.
    pub(crate) fn command_line(&self) -> String {
        std::iter::once("ovis-terminal")
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

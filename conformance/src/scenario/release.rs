//! The `release` step: release-stage runs (todo 67) on the binary under test.

use serde::Deserialize;
use serde_json::Value;

use super::matcher::Expect;

/// Planned release-stage runs; `--inject '{"fixture": <name>}'` selects exactly one.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Release {
    pub(crate) planned: Vec<ReleaseCheck>,
}

/// The in-process tool a release check runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ReleaseTool {
    /// `release-stage` (todo 67).
    #[default]
    ReleaseStage,
    /// `install-probe` (todo 68): the literal `sh install.sh` journeys.
    InstallProbe,
}

/// One release-stage or install-probe run and what it must report.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReleaseCheck {
    /// `{"fixture": <name>}` or a literal command line, compared exactly with `--inject`.
    pub(crate) request: Value,
    /// The tool; `release-stage` when absent.
    #[serde(default)]
    pub(crate) tool: ReleaseTool,
    /// Arguments after the tool name; `$binary`, `$repo` and `$root` are substituted.
    pub(crate) args: Vec<String>,
    pub(crate) exit: u8,
    #[serde(default)]
    pub(crate) report: Expect,
}

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

/// One release-stage run and what it must report.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReleaseCheck {
    /// `{"fixture": <name>}`, compared exactly with `--inject`.
    pub(crate) request: Value,
    /// Arguments after `release-stage`; `$binary`, `$repo` and `$root` are substituted.
    pub(crate) args: Vec<String>,
    pub(crate) exit: u8,
    #[serde(default)]
    pub(crate) report: Expect,
}

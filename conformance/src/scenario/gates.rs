//! The `gates` step: quality tools run in-process on seeded fixtures or the repository.

use serde::Deserialize;
use serde_json::Value;

use super::matcher::Expect;

/// Planned gate runs; `--inject '{"fixture": <name>}'` selects exactly one of them.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Gates {
    pub(crate) planned: Vec<GateCheck>,
}

/// One gate run and what it must report.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GateCheck {
    /// `{"fixture": <name>}`, compared exactly with `--inject`.
    pub(crate) request: Value,
    /// Binary name of the tool, such as `evidence-audit`.
    pub(crate) tool: String,
    /// Arguments instead of `--fixture <name>`; `$repo` becomes the repository root.
    #[serde(default)]
    pub(crate) args: Option<Vec<String>>,
    pub(crate) exit: u8,
    #[serde(default)]
    pub(crate) report: Expect,
}

//! Steps that start, follow and stop the fixture process.

use serde::Deserialize;

use super::matcher::Expect;

/// Fixture servers the runner can start.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FixtureKind {
    /// Real server, dispatcher and store with protocol test methods.
    Protocol,
    /// The binary under test itself: `ovis-terminal serve`.
    Assembled,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Serve {
    pub(crate) fixture: FixtureKind,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Stop {
    pub(crate) expect: Expect,
}

/// The fixture's next stdout line, checked like a reply; `$capture` keeps its values.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Announced {
    pub(crate) expect: Expect,
}

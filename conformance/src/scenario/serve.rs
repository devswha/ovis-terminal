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
    /// The assembled binary's `--qa-graphics-sequence-max-bytes`; its root is marked a QA
    /// fixture root. The protocol fixture refuses it.
    #[serde(default)]
    pub(crate) qa_graphics_sequence_max_bytes: Option<crate::ids::Decimal>,
    /// Assembled only: marks the server root a QA fixture root and passes
    /// `--qa-output-ring-bytes`, so retention overflows after this many output bytes.
    #[serde(default)]
    pub(crate) qa_output_ring_bytes: Option<crate::ids::Decimal>,
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

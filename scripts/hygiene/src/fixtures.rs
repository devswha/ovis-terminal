//! Checked-in sources used to demonstrate each rule on its exact boundary.

/// A named fixture source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Fixture {
    /// Name accepted by `workspace-hygiene check --fixture <name>`.
    pub name: &'static str,
    /// Path reported for findings, relative to the workspace root.
    pub path: &'static str,
    /// Fixture source text.
    pub source: &'static str,
}

/// Every fixture shipped with the checker.
pub const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "boundary-250-loc-three-parameters",
        path: "scripts/hygiene/fixtures/boundary-250-loc-three-parameters.rs",
        source: include_str!("../fixtures/boundary-250-loc-three-parameters.rs"),
    },
    Fixture {
        name: "excessive-loc-and-parameters",
        path: "scripts/hygiene/fixtures/excessive-loc-and-parameters.rs",
        source: include_str!("../fixtures/excessive-loc-and-parameters.rs"),
    },
    Fixture {
        name: "malformed-syntax",
        path: "scripts/hygiene/fixtures/malformed-syntax.rs",
        source: include_str!("../fixtures/malformed-syntax.rs"),
    },
];

/// Looks up a fixture by name.
pub fn find(name: &str) -> Option<&'static Fixture> {
    FIXTURES.iter().find(|fixture| fixture.name == name)
}

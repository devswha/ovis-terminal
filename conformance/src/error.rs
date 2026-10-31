//! Every way a conformance run can fail; each has a stable kind and a recovery hint.

use std::path::PathBuf;

use ovis_terminal_protocol::wire::FrameError;

/// A failed run, scenario, case or step. Exit status 1 unless it is a usage error.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RunnerError {
    /// The command line is incomplete or contradicts itself.
    #[error("the command line is not a runnable conformance invocation: {detail}")]
    Usage { detail: &'static str },
    /// The runner only drives a fresh isolated session root, never the user's own session.
    #[error("the run was not requested with --isolated")]
    IsolationRequired,
    /// The binary under test does not exist or cannot be read.
    #[error("the binary under test cannot be read at {path}")]
    BinaryMissing {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// No scenario file exists for the requested identity.
    #[error("no scenario file exists for {scenario}")]
    ScenarioMissing { scenario: String },
    /// The file is a declaration without runner-executable assertions.
    #[error("{scenario} is a declaration, not a registered executable scenario")]
    ScenarioNotExecutable { scenario: String },
    /// A scenario or fixture file does not match its closed schema.
    #[error("{path} does not match the closed scenario schema at {field}")]
    ScenarioInvalid {
        path: PathBuf,
        field: String,
        #[source]
        source: serde_json::Error,
    },
    /// A scenario's declared identity, cases or includes are inconsistent.
    #[error("{scenario} is inconsistent: {detail}")]
    ScenarioInconsistent {
        scenario: String,
        detail: &'static str,
    },
    /// The requested case is not declared by the scenario.
    #[error("{scenario} declares no {case} case")]
    CaseMissing {
        scenario: String,
        case: &'static str,
    },
    /// A case finished without checking anything.
    #[error("the case ran no assertion")]
    NoAssertions,
    /// An expectation did not hold.
    #[error("expectation {check} did not hold")]
    AssertionFailed { check: String },
    /// A bounded wait expired.
    #[error("{waiting_for} did not happen within {bound_ms} ms")]
    Timeout {
        waiting_for: &'static str,
        bound_ms: u64,
    },
    /// The peer ended the stream while a record was still expected.
    #[error("the connection ended while {waiting_for} was expected")]
    Closed { waiting_for: &'static str },
    /// Bytes from the peer are not a valid record stream.
    #[error("the peer sent an invalid record stream")]
    Wire(#[from] FrameError),
    /// A record is not the JSON the protocol requires.
    #[error("the peer sent a record that is not JSON")]
    Json(#[from] serde_json::Error),
    /// The peer sent something the step did not expect at this point.
    #[error("unexpected record while {waiting_for}")]
    Unexpected { waiting_for: &'static str },
    /// A step names a connection that was never opened or already closed.
    #[error("connection {conn} is not open")]
    UnknownConnection { conn: String },
    /// The scenario needs a fixture server that has not been started, or starts a second one.
    #[error("the fixture server is not in the state this step needs")]
    FixtureState,
    /// The fixture server failed before reporting readiness or its final report.
    #[error("the fixture server did not report {expected}")]
    FixtureReport { expected: &'static str },
    /// A process could not be started.
    #[error("{program} could not be started")]
    Spawn {
        program: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Local I/O failed.
    #[error("{operation} failed")]
    Io {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
    /// This platform has no listener the fixture can serve.
    #[error("the fixture server cannot serve on this platform")]
    UnsupportedPlatform,
    /// A counter in a report is not a canonical decimal string.
    #[error("a counter is not a canonical decimal string")]
    Decimal,
    /// The in-process fixture server failed.
    #[error("the fixture server failed to {stage}")]
    Fixture {
        stage: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl RunnerError {
    /// Stable machine token.
    pub(crate) const fn kind(&self) -> &'static str {
        match self {
            Self::Usage { .. } => "usage",
            Self::IsolationRequired => "isolation_required",
            Self::BinaryMissing { .. } => "binary_missing",
            Self::ScenarioMissing { .. } => "scenario_missing",
            Self::ScenarioNotExecutable { .. } => "scenario_not_executable",
            Self::ScenarioInvalid { .. } => "scenario_invalid",
            Self::ScenarioInconsistent { .. } => "scenario_inconsistent",
            Self::CaseMissing { .. } => "case_missing",
            Self::NoAssertions => "no_assertions",
            Self::AssertionFailed { .. } => "assertion_failed",
            Self::Timeout { .. } => "timeout",
            Self::Closed { .. } => "closed",
            Self::Wire(_) => "wire",
            Self::Json(_) => "json",
            Self::Unexpected { .. } => "unexpected_record",
            Self::UnknownConnection { .. } => "unknown_connection",
            Self::FixtureState => "fixture_state",
            Self::FixtureReport { .. } => "fixture_report",
            Self::Spawn { .. } => "spawn_failed",
            Self::Io { .. } => "io",
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::Decimal => "invalid_decimal",
            Self::Fixture { .. } => "fixture_failed",
        }
    }

    /// Recovery guidance.
    pub(crate) const fn hint(&self) -> &'static str {
        match self {
            Self::Usage { .. } => "pass --binary, --scenario, --isolated and --evidence",
            Self::IsolationRequired => "add --isolated; the runner never touches a live session",
            Self::BinaryMissing { .. } => {
                "build the binary first: cargo build -p ovis-terminal-cli"
            }
            Self::ScenarioMissing { .. } => "name a file under conformance/scenarios/<family>/",
            Self::ScenarioNotExecutable { .. } => {
                "register the scenario with \"format\": \"ovis-conformance/1\" and typed steps"
            }
            Self::ScenarioInvalid { .. } | Self::ScenarioInconsistent { .. } => {
                "fix the scenario file; unknown keys are never ignored"
            }
            Self::CaseMissing { .. } => "request a case the scenario declares, or --case all",
            Self::NoAssertions => "add at least one expectation to the case",
            Self::AssertionFailed { .. } => "compare expected and actual values in the evidence",
            Self::Timeout { .. } | Self::Closed { .. } | Self::Unexpected { .. } => {
                "inspect the byte transcript in the evidence"
            }
            Self::Wire(_) | Self::Json(_) => "inspect the offending bytes in the transcript",
            Self::UnknownConnection { .. } => "open the connection with a connect step first",
            Self::FixtureState => "start one fixture with a serve step before using it",
            Self::FixtureReport { .. } | Self::Fixture { .. } => {
                "inspect the fixture's stderr in the evidence"
            }
            Self::Spawn { .. } | Self::Io { .. } => "check paths and permissions on this host",
            Self::UnsupportedPlatform => "run this scenario on a platform it declares",
            Self::Decimal => "encode counters as canonical decimal strings",
        }
    }
}

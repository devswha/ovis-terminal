//! Writing a run's evidence file, its one-line summary and the manual transcript.

use std::io::Write as _;

use crate::error::RunnerError;
use crate::evidence::{CaseReport, Evidence};

pub(crate) fn write_evidence(
    path: &std::path::Path,
    evidence: &Evidence,
) -> Result<(), RunnerError> {
    let io = |source| RunnerError::Io {
        operation: "write the evidence file",
        source,
    };
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    let bytes = serde_json::to_vec_pretty(evidence)?;
    std::fs::write(path, bytes).map_err(io)
}

pub(crate) fn summary(value: &serde_json::Value) {
    let mut stdout = std::io::stdout().lock();
    if writeln!(stdout, "{value}").is_err() {
        drop(writeln!(
            std::io::stderr().lock(),
            "conformance: stdout is closed"
        ));
    }
}

pub(crate) fn transcript(cases: &[CaseReport]) {
    let mut stderr = std::io::stderr().lock();
    for case in cases {
        for conn in &case.connections {
            for record in &conn.transcript {
                let direction = match record.direction {
                    crate::evidence::Direction::Sent => ">>",
                    crate::evidence::Direction::Received => "<<",
                };
                let text = record.text.as_deref().unwrap_or("<elided: see sha256>");
                let line = writeln!(
                    stderr,
                    "{}/{} {} {direction} @{} {}B {}: {}",
                    case.scenario,
                    case.case.as_str(),
                    conn.name,
                    record.offset.0,
                    record.bytes.0,
                    record.unit,
                    text.trim_end()
                );
                if line.is_err() {
                    return;
                }
            }
        }
    }
}

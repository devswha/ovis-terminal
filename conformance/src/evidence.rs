//! The evidence file: requests, replies, byte transcripts, processes and every assertion.

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::ids::{CaseName, Decimal, ScenarioId};
use crate::scenario::matcher::Assertion;

/// Free-text fixture documentation: recorded in the evidence, never asserted.
pub(crate) type Descriptors = std::collections::BTreeMap<String, String>;

/// The descriptors of one fixture a case used, and where they came from.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct DescriptorRecord {
    pub(crate) source: String,
    pub(crate) descriptors: Descriptors,
}

/// Record text longer than this is kept as length and digest only.
pub(crate) const TEXT_LIMIT: usize = 4096;

/// Lowercase hex without separators.
pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        text.extend(char::from_digit(u32::from(byte >> 4), 16));
        text.extend(char::from_digit(u32::from(byte & 0x0f), 16));
    }
    text
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// The whole run.
#[derive(Debug, Serialize)]
pub(crate) struct Evidence {
    pub(crate) format: &'static str,
    pub(crate) runner_version: &'static str,
    pub(crate) host: Host,
    pub(crate) binary: Option<BinaryRecord>,
    pub(crate) scenario: ScenarioId,
    pub(crate) case: &'static str,
    pub(crate) result: Outcome,
    pub(crate) totals: Totals,
    /// Why the run failed as a whole: a missing scenario or binary, an invalid or unlisted
    /// scenario file, a stale allowlist entry, or declarations under `--require-no-skips`.
    pub(crate) error: Option<Failure>,
    /// For a suite: every file under the scenario root, classified.
    pub(crate) inventory: Option<crate::scenario::inventory::Inventory>,
    pub(crate) elapsed_ms: Decimal,
    pub(crate) cases: Vec<CaseReport>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Host {
    pub(crate) os: &'static str,
    pub(crate) arch: &'static str,
    pub(crate) family: &'static str,
}

impl Host {
    pub(crate) const fn current() -> Self {
        Self {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            family: std::env::consts::FAMILY,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct BinaryRecord {
    pub(crate) path: String,
    pub(crate) sha256: String,
    pub(crate) bytes: Decimal,
}

/// Pass, fail or skip; a skip never counts as a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct Totals {
    pub(crate) cases: Decimal,
    pub(crate) passed: Decimal,
    pub(crate) failed: Decimal,
    pub(crate) skipped: Decimal,
    /// Declaration files the suite found and did not execute.
    pub(crate) declared_only: Decimal,
    pub(crate) assertions: Decimal,
}

impl Totals {
    pub(crate) fn of(cases: &[CaseReport]) -> Self {
        let count = |outcome| {
            Decimal::from_usize(cases.iter().filter(|case| case.outcome == outcome).count())
        };
        Self {
            cases: Decimal::from_usize(cases.len()),
            passed: count(Outcome::Passed),
            failed: count(Outcome::Failed),
            skipped: count(Outcome::Skipped),
            declared_only: Decimal(0),
            assertions: Decimal::from_usize(cases.iter().map(|case| case.assertions.len()).sum()),
        }
    }
}

/// The typed reason a case did not pass.
#[derive(Debug, Serialize)]
pub(crate) struct Failure {
    pub(crate) kind: &'static str,
    pub(crate) hint: &'static str,
    pub(crate) detail: String,
    pub(crate) step: Option<Decimal>,
}

/// One case of one scenario.
#[derive(Debug, Serialize)]
pub(crate) struct CaseReport {
    pub(crate) scenario: ScenarioId,
    pub(crate) case: CaseName,
    pub(crate) summary: String,
    /// What the scenario file declares it proves, copied for traceability audits.
    pub(crate) declared: Declared,
    pub(crate) outcome: Outcome,
    pub(crate) failure: Option<Failure>,
    pub(crate) assertions: Vec<Assertion>,
    pub(crate) descriptors: Vec<DescriptorRecord>,
    pub(crate) connections: Vec<ConnectionReport>,
    pub(crate) processes: Vec<ProcessReport>,
    pub(crate) fixture: Option<Value>,
    pub(crate) elapsed_ms: Decimal,
}

/// A scenario's owner and the ledger ids it declares.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Declared {
    pub(crate) title: String,
    pub(crate) todo: String,
    pub(crate) closes: Vec<String>,
    pub(crate) delivers: Vec<String>,
    pub(crate) excludes: Vec<String>,
}

/// Direction of a transcript record.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Direction {
    Sent,
    Received,
}

/// One write or one decoded record, with its absolute stream offset.
#[derive(Debug, Serialize)]
pub(crate) struct Record {
    pub(crate) direction: Direction,
    pub(crate) offset: Decimal,
    pub(crate) bytes: Decimal,
    pub(crate) sha256: String,
    pub(crate) at_ms: Decimal,
    /// `line`, `frame:<channel>:<kind>` or `write`.
    pub(crate) unit: String,
    pub(crate) text: Option<String>,
}

/// Byte accounting and transcript of one connection.
#[derive(Debug, Default, Serialize)]
pub(crate) struct ConnectionReport {
    pub(crate) name: String,
    pub(crate) via: String,
    pub(crate) bytes_sent: Decimal,
    pub(crate) writes: Decimal,
    pub(crate) bytes_received: Decimal,
    pub(crate) bytes_decoded: Decimal,
    pub(crate) records_received: Decimal,
    pub(crate) transcript: Vec<Record>,
}

/// One process the runner started and reaped.
#[derive(Debug, Serialize)]
pub(crate) struct ProcessReport {
    pub(crate) role: &'static str,
    pub(crate) argv: Vec<String>,
    pub(crate) pid: Option<Decimal>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) stdout: CapturedText,
    pub(crate) stderr: CapturedText,
}

/// Captured output: text when short, always length and digest.
#[derive(Debug, Serialize)]
pub(crate) struct CapturedText {
    pub(crate) bytes: Decimal,
    pub(crate) sha256: String,
    pub(crate) text: Option<String>,
}

impl CapturedText {
    pub(crate) fn of(bytes: &[u8]) -> Self {
        Self {
            bytes: Decimal::from_usize(bytes.len()),
            sha256: sha256_hex(bytes),
            text: (bytes.len() <= TEXT_LIMIT).then(|| String::from_utf8_lossy(bytes).into_owned()),
        }
    }
}

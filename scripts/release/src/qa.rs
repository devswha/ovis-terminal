//! `qa`: on the target's own host, unpack the sealed archive as an installer would and run
//! the unpacked binary. The receipt is written whether or not every check holds.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::binary::{compat, identity};
use crate::docs::{Entry, QaCheck, QaReceipt};
use crate::error::ReleaseError;
use crate::files::{read_json, sha256_bytes, unpack, write_json};
use crate::seal::{verify_archive, verify_files};
use crate::target::{Platform, binary_name};

/// Inputs of one native QA run.
#[derive(Debug, Clone)]
pub(crate) struct QaInput {
    /// The sealed entry.
    pub(crate) entry: PathBuf,
    /// An empty directory to unpack into.
    pub(crate) install_dir: PathBuf,
    /// The released baseline the unpacked binary is compared with.
    pub(crate) released: PathBuf,
    /// The runner label, or `local`.
    pub(crate) runner: String,
}

fn check(name: &str, outcome: Result<Value, ReleaseError>) -> QaCheck {
    match outcome {
        Ok(detail) => QaCheck {
            check: name.to_owned(),
            passed: true,
            detail,
        },
        Err(error) => QaCheck {
            check: name.to_owned(),
            passed: false,
            detail: json!({ "kind": error.kind(), "message": error.to_string(), "details": error.details() }),
        },
    }
}

fn installed_checks(entry: &Entry, installed: &Path, released: &Path) -> Vec<QaCheck> {
    let binary = installed.join(&entry.name).join(binary_name(&entry.target));
    let mut checks = vec![check(
        "files_match_entry",
        verify_files(installed, entry).map(|()| json!({})),
    )];
    let found = identity(&binary);
    let schema_record = entry
        .files
        .iter()
        .find(|file| file.path == "ovis-terminal.openrpc.json");
    checks.push(check(
        "version_and_manual",
        found.as_ref().map_err(clone_kind).and_then(|found| {
            let agreed = [&found.version, &found.manual_version]
                .iter()
                .all(|seen| **seen == entry.version);
            if agreed {
                Ok(json!({ "version": found.version, "generation": found.generation }))
            } else {
                Err(ReleaseError::VersionMismatch {
                    part: "the unpacked binary",
                    expected: entry.version.clone(),
                    found: format!("{} / {}", found.version, found.manual_version),
                })
            }
        }),
    ));
    checks.push(check(
        "schema_equals_staged",
        found.as_ref().map_err(clone_kind).and_then(|found| {
            let digest = sha256_bytes(&found.schema);
            if schema_record.is_some_and(|record| record.sha256 == digest) {
                Ok(json!({ "sha256": digest }))
            } else {
                Err(ReleaseError::SchemaMismatch)
            }
        }),
    ));
    checks.push(check("released_schema_compat", compat(&binary, released)));
    checks
}

/// A failed identity read, restated for the checks that depend on it.
fn clone_kind(error: &ReleaseError) -> ReleaseError {
    ReleaseError::Tool {
        program: "ovis-terminal discovery".to_owned(),
        detail: format!("{} ({})", error, error.kind()),
    }
}

/// Runs native QA and writes `<name>.qa.json` beside the entry.
///
/// # Errors
///
/// [`ReleaseError::NotNative`] off the target's host (nothing is written), or
/// [`ReleaseError::NativeQaFailed`] after writing a receipt with a failed check.
pub(crate) fn qa(input: &QaInput) -> Result<Value, ReleaseError> {
    let entry: Entry = read_json(&input.entry, "release entry")?;
    let host = Platform::host();
    if Platform::of_triple(&entry.target)? != host {
        return Err(ReleaseError::NotNative {
            target: entry.target,
            host: host.label(),
        });
    }
    let dir = input.entry.parent().unwrap_or_else(|| Path::new("."));
    let archive = verify_archive(dir, &entry)?;
    unpack(&dir.join(&archive.file), &input.install_dir)?;
    let mut checks = vec![check(
        "archive_checksum",
        Ok(json!({ "sha256": archive.sha256 })),
    )];
    checks.extend(installed_checks(
        &entry,
        &input.install_dir,
        &input.released,
    ));
    let failed: Vec<String> = checks
        .iter()
        .filter(|c| !c.passed)
        .map(|c| c.check.clone())
        .collect();
    let receipt = QaReceipt {
        format: "ovis-release-qa/1".to_owned(),
        name: entry.name.clone(),
        target: entry.target.clone(),
        host,
        runner: input.runner.clone(),
        archive_sha256: archive.sha256,
        passed: failed.is_empty(),
        checks,
    };
    let path = dir.join(format!("{}.qa.json", entry.name));
    write_json(&path, &receipt)?;
    if failed.is_empty() {
        Ok(json!({ "qa": entry.name, "passed": true, "checks": receipt.checks.len().to_string() }))
    } else {
        Err(ReleaseError::NativeQaFailed {
            target: entry.target,
            failed,
        })
    }
}

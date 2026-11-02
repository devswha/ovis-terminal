//! A synthetic five-target staging directory for the promotion fixtures.
//!
//! Each target has a sealed archive with its `.sha256` sidecar, a pinned native receipt and a
//! passing native QA receipt; the staged directories are removed, exactly as the release
//! workflow uploads only entries, archives, sidecars and QA receipts to the promote job. Its
//! binaries are placeholder bytes; only the promotion gate, never a binary, is under test.

use std::path::Path;

use serde_json::json;

use crate::docs::{Checks, Entry, Pins, QaCheck, QaReceipt, Receipt};
use crate::error::ReleaseError;
use crate::files::write_json;
use crate::seal::seal;
use crate::stage::records;
use crate::target::{Platform, RELEASE_TARGETS, binary_name};

/// The version every synthetic artifact carries.
pub(crate) const VERSION: &str = "0.0.0-fixture";

fn receipt(target: &str, pins: &Pins) -> Result<Receipt, ReleaseError> {
    let pin = pins
        .targets
        .get(target)
        .ok_or_else(|| ReleaseError::UnknownTarget {
            triple: target.to_owned(),
        })?;
    Ok(Receipt {
        format: "ovis-release-receipt/1".to_owned(),
        target: target.to_owned(),
        host: Platform::of_triple(target)?,
        runner: pin.runner.clone(),
        rustc: pins.rust.clone(),
        sqlite: pins.sqlite.clone(),
        libsqlite3_sys: pins.libsqlite3_sys.clone(),
        c_toolchain: pin.c_toolchain.clone(),
    })
}

fn one(dir: &Path, pins: &Pins, (target, salt): (&str, &str)) -> Result<(), ReleaseError> {
    let name = format!("ovis-terminal-{VERSION}-{target}");
    let artifact = dir.join(&name);
    std::fs::create_dir_all(&artifact).map_err(ReleaseError::io("create", &artifact))?;
    let binary = artifact.join(binary_name(target));
    std::fs::write(&binary, format!("synthetic {target} {salt}\n"))
        .map_err(ReleaseError::io("write", &binary))?;
    let receipt = receipt(target, pins)?;
    write_json(&artifact.join("receipt.json"), &receipt)?;
    let entry_path = dir.join(format!("{name}.entry.json"));
    write_json(
        &entry_path,
        &Entry {
            format: "ovis-release-entry/1".to_owned(),
            name: name.clone(),
            version: VERSION.to_owned(),
            generation: "1".to_owned(),
            target: target.to_owned(),
            release_target: true,
            files: records(&artifact)?,
            checks: Checks {
                schema_matches_source: true,
                versions: [("binary".to_owned(), VERSION.to_owned())].into(),
                compat: json!({ "exit": 0, "synthetic": true }),
                notice_crates: "0".to_owned(),
                apache_attribution: true,
            },
            receipt: receipt.clone(),
            archive: None,
        },
    )?;
    let sealed = seal(&entry_path)?;
    std::fs::remove_dir_all(&artifact).map_err(ReleaseError::io("remove", &artifact))?;
    let qa = QaReceipt {
        format: "ovis-release-qa/1".to_owned(),
        name: name.clone(),
        target: target.to_owned(),
        host: receipt.host,
        runner: receipt.runner,
        archive_sha256: sealed
            .get("sha256")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        checks: vec![QaCheck {
            check: "synthetic".to_owned(),
            passed: true,
            detail: json!({}),
        }],
        passed: true,
    };
    write_json(&dir.join(format!("{name}.qa.json")), &qa)
}

/// Writes all five synthetic targets into `dir`; `salt` changes every binary's bytes.
pub(crate) fn build(dir: &Path, pins: &Pins, salt: &str) -> Result<(), ReleaseError> {
    std::fs::create_dir_all(dir).map_err(ReleaseError::io("create", dir))?;
    RELEASE_TARGETS
        .iter()
        .try_for_each(|target| one(dir, pins, (target, salt)))
}

/// The entry and QA receipt paths of `target` in a synthetic directory.
pub(crate) fn paths(dir: &Path, target: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let name = format!("ovis-terminal-{VERSION}-{target}");
    (
        dir.join(format!("{name}.entry.json")),
        dir.join(format!("{name}.qa.json")),
    )
}

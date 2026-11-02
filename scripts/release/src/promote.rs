//! `promote`: the orchestrator-only gate before publication.
//!
//! It requires all five native artifacts, their native QA receipts, pinned build receipts, the notices and a compatible
//! released schema, then writes `release-manifest.json` and `SHA256SUMS`. It decides whether
//! to publish; it never uploads, and a version is published at most once.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::docs::{Entry, Manifest, ManifestTarget, Pins, QaReceipt, Receipt};
use crate::error::ReleaseError;
use crate::files::{read_json, sha256_bytes, sha256_file, write_json};
use crate::seal::{entries, verify_archive, verify_files};
use crate::target::{Platform, RELEASE_TARGETS};

/// Inputs of one promotion.
#[derive(Debug, Clone)]
pub(crate) struct PromoteInput {
    /// The directory holding every target's entry, archive and QA receipt.
    pub(crate) dir: PathBuf,
    /// The version being released.
    pub(crate) version: String,
    /// `scripts/release/pins.json`.
    pub(crate) pins: PathBuf,
    /// The manifest already published for this version, if any.
    pub(crate) existing: Option<PathBuf>,
    /// Decide and write the manifest, but never publish.
    pub(crate) dry_run: bool,
}

fn pinned(target: &str, field: &str, (pin, found): (&str, &str)) -> Result<(), ReleaseError> {
    if pin == found {
        return Ok(());
    }
    Err(ReleaseError::ReceiptUnpinned {
        target: target.to_owned(),
        field: field.to_owned(),
        pinned: pin.to_owned(),
        found: found.to_owned(),
    })
}

/// Requires a receipt built natively with exactly the pinned toolchain.
///
/// # Errors
///
/// [`ReleaseError::CrossBuildRefused`] or [`ReleaseError::ReceiptUnpinned`].
pub(crate) fn check_receipt(receipt: &Receipt, pins: &Pins) -> Result<(), ReleaseError> {
    let target = receipt.target.as_str();
    if receipt.host != Platform::of_triple(target)? {
        return Err(ReleaseError::CrossBuildRefused {
            target: target.to_owned(),
            host: receipt.host.label(),
        });
    }
    let pin = pins
        .targets
        .get(target)
        .ok_or_else(|| ReleaseError::UnknownTarget {
            triple: target.to_owned(),
        })?;
    pinned(target, "rustc", (&pins.rust, &receipt.rustc))?;
    pinned(target, "sqlite", (&pins.sqlite, &receipt.sqlite))?;
    pinned(
        target,
        "libsqlite3_sys",
        (&pins.libsqlite3_sys, &receipt.libsqlite3_sys),
    )?;
    pinned(target, "runner", (&pin.runner, &receipt.runner))?;
    for (name, value) in &pin.c_toolchain {
        let found = receipt.c_toolchain.get(name).map_or("", String::as_str);
        pinned(target, &format!("c_toolchain.{name}"), (value, found))?;
    }
    Ok(())
}

fn checked_target(
    input: &PromoteInput,
    pins: &Pins,
    entry: &Entry,
) -> Result<ManifestTarget, ReleaseError> {
    if entry.version != input.version {
        return Err(ReleaseError::VersionMismatch {
            part: "an artifact entry",
            expected: input.version.clone(),
            found: entry.version.clone(),
        });
    }
    check_receipt(&entry.receipt, pins)?;
    if !entry.checks.apache_attribution {
        return Err(ReleaseError::NoticesMissing {
            missing: "the Apache-2.0 attribution in an artifact",
        });
    }
    if entry.checks.compat.get("exit") != Some(&json!(0)) {
        return Err(ReleaseError::CompatibilityBreak {
            binary: entry.checks.compat.clone(),
        });
    }
    verify_files(&input.dir, entry)?;
    let archive = verify_archive(&input.dir, entry)?;
    let qa_path = input.dir.join(format!("{}.qa.json", entry.name));
    if !qa_path.is_file() {
        return Err(ReleaseError::NativeQaMissing {
            target: entry.target.clone(),
        });
    }
    let qa: QaReceipt = read_json(&qa_path, "native QA receipt")?;
    let platform = Platform::of_triple(&entry.target)?;
    let mut failed: Vec<String> = qa
        .checks
        .iter()
        .filter(|c| !c.passed)
        .map(|c| c.check.clone())
        .collect();
    if qa.host != platform {
        failed.push(format!("host {} is not native", qa.host.label()));
    }
    if qa.archive_sha256 != archive.sha256 {
        failed.push("receipt is for another archive".to_owned());
    }
    if qa.target != entry.target {
        failed.push("receipt is for another target".to_owned());
    }
    if !qa.passed {
        failed.push("receipt does not pass".to_owned());
    }
    if !failed.is_empty() {
        return Err(ReleaseError::NativeQaFailed {
            target: entry.target.clone(),
            failed,
        });
    }
    Ok(ManifestTarget {
        target: entry.target.clone(),
        platform,
        archive: archive.file,
        sha256: archive.sha256,
        bytes: archive.bytes,
        runner: entry.receipt.runner.clone(),
        qa_sha256: sha256_file(&qa_path)?.0,
    })
}

/// The manifest every native target supports, in release order.
///
/// # Errors
///
/// [`ReleaseError::TargetMissing`] naming every absent target, then the first refused target.
pub(crate) fn manifest(input: &PromoteInput, pins: &Pins) -> Result<Manifest, ReleaseError> {
    let staged: BTreeMap<String, Entry> = entries(&input.dir)?
        .into_iter()
        .filter(|entry| entry.release_target)
        .map(|entry| (entry.target.clone(), entry))
        .collect();
    let missing: Vec<String> = RELEASE_TARGETS
        .iter()
        .filter(|target| !staged.contains_key(**target))
        .map(|target| (*target).to_owned())
        .collect();
    if !missing.is_empty() {
        return Err(ReleaseError::TargetMissing { targets: missing });
    }
    let targets = RELEASE_TARGETS
        .iter()
        .filter_map(|target| staged.get(*target))
        .map(|entry| checked_target(input, pins, entry))
        .collect::<Result<Vec<_>, _>>()?;
    let generation = staged
        .values()
        .next()
        .map(|entry| entry.generation.clone())
        .unwrap_or_default();
    Ok(Manifest {
        format: "ovis-release-manifest/1".to_owned(),
        version: input.version.clone(),
        generation,
        signing: pins.signing.clone(),
        targets,
    })
}

fn write_outputs(dir: &Path, manifest: &Manifest) -> Result<String, ReleaseError> {
    write_json(&dir.join("release-manifest.json"), manifest)?;
    let sums = manifest
        .targets
        .iter()
        .map(|target| [target.sha256.as_str(), "  ", target.archive.as_str(), "\n"].concat())
        .collect::<String>();
    let path = dir.join("SHA256SUMS");
    std::fs::write(&path, sums).map_err(ReleaseError::io("write", &path))?;
    let compact = serde_json::to_vec(manifest).map_err(|source| ReleaseError::Malformed {
        path: dir.join("release-manifest.json"),
        document: "release manifest",
        source,
    })?;
    Ok(sha256_bytes(&compact))
}

/// Gates, writes the manifest and decides: `promote` (publish), `dry_run` or
/// `already_published` (an identical manifest is already out; nothing is published twice).
///
/// # Errors
///
/// Any refused target, or [`ReleaseError::ReleaseConflict`] when this version was
/// published with a different manifest.
pub(crate) fn promote(input: &PromoteInput) -> Result<Value, ReleaseError> {
    let pins: Pins = read_json(&input.pins, "release pins")?;
    let manifest = manifest(input, &pins)?;
    let existing: Option<Manifest> = input
        .existing
        .as_deref()
        .map(|path| read_json::<Manifest>(path, "release manifest"))
        .transpose()?
        .filter(|published| published.version == input.version);
    if existing
        .as_ref()
        .is_some_and(|published| *published != manifest)
    {
        return Err(ReleaseError::ReleaseConflict {
            version: input.version.clone(),
        });
    }
    let digest = write_outputs(&input.dir, &manifest)?;
    let outcome = match (existing.is_some(), input.dry_run) {
        (true, _) => "already_published",
        (false, true) => "dry_run",
        (false, false) => "promote",
    };
    let supported: Vec<String> = manifest
        .targets
        .iter()
        .map(|t| t.platform.label())
        .collect();
    Ok(json!({
        "outcome": outcome,
        "publish": outcome == "promote",
        "version": manifest.version,
        "targets": manifest.targets.len().to_string(),
        "supported": supported,
        "manifest_sha256": digest,
        "signing": manifest.signing,
    }))
}

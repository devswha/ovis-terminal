//! Promotion-gate fixtures over the synthetic five-target matrix.
//!
//! Each seeds one defect (a missing target or receipt, a foreign host, an unpinned
//! toolchain, a bad digest, an unknown field) or a rerun, then runs the gate.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::docs::{ArchiveRecord, Entry, Pins, QaReceipt};
use crate::error::ReleaseError;
use crate::files::{pack, read_json, sha256_file, unpack, write_json};
use crate::fixtures::{Paths, outcome};
use crate::matrix::{self, VERSION};
use crate::promote::{PromoteInput, promote};
use crate::seal::{sidecar_line, verify};
use crate::target::{Platform, binary_name};

fn promotion(
    dir: &Path,
    pins: &Path,
    (existing, dry_run): (Option<PathBuf>, bool),
) -> Result<Value, ReleaseError> {
    promote(&PromoteInput {
        dir: dir.to_path_buf(),
        version: VERSION.to_owned(),
        pins: pins.to_path_buf(),
        existing,
        dry_run,
    })
}

fn rewrite_entry(path: &Path, change: impl FnOnce(&mut Entry)) -> Result<(), ReleaseError> {
    let mut entry: Entry = read_json(path, "release entry")?;
    change(&mut entry);
    write_json(path, &entry)
}

fn rewrite_qa(path: &Path, change: impl FnOnce(&mut QaReceipt)) -> Result<(), ReleaseError> {
    let mut qa: QaReceipt = read_json(path, "native QA receipt")?;
    change(&mut qa);
    write_json(path, &qa)
}

fn archive_of(dir: &Path, target: &str) -> PathBuf {
    dir.join(format!("ovis-terminal-{VERSION}-{target}.tar.gz"))
}

/// Rebuilds `target`'s archive with a tampered binary and re-records every digest of it
/// (entry archive record, sidecar, QA receipt), leaving only the per-file records stale.
fn repack(dir: &Path, target: &str) -> Result<(), ReleaseError> {
    let name = format!("ovis-terminal-{VERSION}-{target}");
    unpack(&archive_of(dir, target), dir)?;
    let binary = dir.join(&name).join(binary_name(target));
    std::fs::write(&binary, b"tampered\n").map_err(ReleaseError::io("write", &binary))?;
    let file = pack(dir, &name)?;
    let (sha256, bytes) = sha256_file(&dir.join(&file))?;
    let sidecar = dir.join(format!("{file}.sha256"));
    std::fs::write(&sidecar, sidecar_line(&sha256, &file))
        .map_err(ReleaseError::io("write", &sidecar))?;
    let (entry, qa) = matrix::paths(dir, target);
    rewrite_entry(&entry, |entry| {
        entry.archive = Some(ArchiveRecord {
            file,
            sha256: sha256.clone(),
            bytes,
        });
    })?;
    rewrite_qa(&qa, |qa| qa.archive_sha256 = sha256)?;
    let staged = dir.join(&name);
    std::fs::remove_dir_all(&staged).map_err(ReleaseError::io("remove", &staged))
}

fn remove(path: &Path) -> Result<(), ReleaseError> {
    std::fs::remove_file(path).map_err(ReleaseError::io("remove", path))
}

fn reruns(dir: &Path, pins: &Path, work: &Path) -> Result<(u8, Value), ReleaseError> {
    let first = promotion(dir, pins, (None, false))?;
    let published = work.join("published-manifest.json");
    crate::files::copy(&dir.join("release-manifest.json"), &published)?;
    let (exit, second) = outcome(promotion(dir, pins, (Some(published), false)));
    Ok((exit, json!({ "runs": [first, second] })))
}

pub(crate) fn matrix_fixture(
    name: &str,
    paths: &Paths,
    work: &Path,
) -> Result<(u8, Value), ReleaseError> {
    let pins_path = paths.repo.join("scripts").join("release").join("pins.json");
    let pins: Pins = read_json(&pins_path, "release pins")?;
    let dir = work.join("matrix");
    matrix::build(&dir, &pins, "first")?;
    let entry = |target: &str| matrix::paths(&dir, target);
    let gate = |dry_run| outcome(promotion(&dir, &pins_path, (None, dry_run)));
    Ok(match name {
        "complete-matrix-dry-run" => gate(true),
        "missing-windows-artifact" => {
            remove(&entry("x86_64-pc-windows-msvc").0)?;
            gate(true)
        }
        "missing-native-qa" => {
            remove(&entry("aarch64-apple-darwin").1)?;
            gate(true)
        }
        "cross-built-macos" => {
            rewrite_entry(&entry("x86_64-apple-darwin").0, |entry| {
                entry.receipt.host = Platform {
                    os: "linux".to_owned(),
                    arch: "x86_64".to_owned(),
                };
            })?;
            gate(true)
        }
        "unpinned-sqlite" => {
            rewrite_entry(&entry("x86_64-unknown-linux-musl").0, |entry| {
                "3.0.0".clone_into(&mut entry.receipt.sqlite);
            })?;
            gate(true)
        }
        "bad-manifest-hash" => {
            let archive = archive_of(&dir, "x86_64-pc-windows-msvc");
            std::fs::write(&archive, b"tampered\n").map_err(ReleaseError::io("write", &archive))?;
            outcome(verify(&dir))
        }
        "wrong-sidecar" => {
            let sidecar = dir.join(format!(
                "ovis-terminal-{VERSION}-aarch64-apple-darwin.tar.gz.sha256"
            ));
            std::fs::write(&sidecar, format!("{}  x\n", "0".repeat(64)))
                .map_err(ReleaseError::io("write", &sidecar))?;
            outcome(verify(&dir))
        }
        "failed-native-qa" | "forged-qa-pass" => {
            let forged = name == "forged-qa-pass";
            rewrite_qa(&entry("x86_64-unknown-linux-musl").1, |qa| {
                qa.checks.iter_mut().for_each(|check| check.passed = false);
                qa.passed = forged;
            })?;
            gate(true)
        }
        "repacked-archive" => {
            repack(&dir, "x86_64-unknown-linux-musl")?;
            gate(true)
        }
        "malformed-manifest" => {
            let path = entry("aarch64-unknown-linux-musl").0;
            let mut document: Value = read_json(&path, "release entry")?;
            if let Some(fields) = document.as_object_mut() {
                fields.insert("published".to_owned(), json!(true));
            }
            write_json(&path, &document)?;
            outcome(verify(&dir))
        }
        "rerun-after-publish" => reruns(&dir, &pins_path, work)?,
        "conflicting-rerun" => {
            promotion(&dir, &pins_path, (None, false))?;
            let published = work.join("published-manifest.json");
            crate::files::copy(&dir.join("release-manifest.json"), &published)?;
            matrix::build(&dir, &pins, "rebuilt")?;
            outcome(promotion(&dir, &pins_path, (Some(published), false)))
        }
        _ => {
            return Err(ReleaseError::UnknownFixture {
                name: name.to_owned(),
            });
        }
    })
}

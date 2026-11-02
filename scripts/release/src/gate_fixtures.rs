//! Promotion-gate fixtures over the synthetic five-target matrix.
//!
//! Each seeds one defect (a missing target or receipt, a foreign host, an unpinned
//! toolchain, a bad digest, an unknown field) or a rerun, then runs the gate.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::docs::{Entry, Pins};
use crate::error::ReleaseError;
use crate::files::{read_json, write_json};
use crate::fixtures::{Paths, outcome};
use crate::matrix::{self, VERSION};
use crate::promote::{PromoteInput, promote};
use crate::seal::verify;
use crate::target::Platform;

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
            let binary = dir
                .join(format!("ovis-terminal-{VERSION}-x86_64-pc-windows-msvc"))
                .join("ovis-terminal.exe");
            std::fs::write(&binary, b"tampered\n").map_err(ReleaseError::io("write", &binary))?;
            outcome(verify(&dir))
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

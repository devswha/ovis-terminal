//! `stage`: check one built binary against the source tree, then lay out its artifact.
//! Every check runs before anything is written, so a refused stage leaves no entry.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::binary::{Identity, compat, identity};
use crate::docs::{Checks, Entry, FileRecord, Receipt};
use crate::error::ReleaseError;
use crate::files::{copy, read_json, sha256_file, write_json};
use crate::notices;
use crate::target::{Platform, binary_name, is_release_target};

/// Inputs of one stage.
#[derive(Debug, Clone)]
pub(crate) struct StageInput {
    /// The target triple.
    pub(crate) target: String,
    /// The built binary.
    pub(crate) binary: PathBuf,
    /// The packed TypeScript SDK, `ovis-terminal-client-<version>.tgz`.
    pub(crate) sdk: PathBuf,
    /// The build receipt.
    pub(crate) receipt: PathBuf,
    /// The source tree.
    pub(crate) repo: PathBuf,
    /// The staging directory.
    pub(crate) out: PathBuf,
    /// The released baseline; `<repo>/schema/released.openrpc.json` without it.
    pub(crate) released: Option<PathBuf>,
    /// The notices file; `<repo>/THIRD_PARTY_NOTICES.md` without it.
    pub(crate) notices: Option<PathBuf>,
}

impl StageInput {
    fn released(&self) -> PathBuf {
        self.released
            .clone()
            .unwrap_or_else(|| self.repo.join("schema").join("released.openrpc.json"))
    }

    fn notices(&self) -> PathBuf {
        self.notices
            .clone()
            .unwrap_or_else(|| self.repo.join("THIRD_PARTY_NOTICES.md"))
    }
}

/// A build receipt for `target` that names its own native host.
///
/// # Errors
///
/// [`ReleaseError::ReceiptUnpinned`] for another target, or
/// [`ReleaseError::CrossBuildRefused`] for a receipt from another OS or architecture.
pub(crate) fn native_receipt(path: &Path, target: &str) -> Result<Receipt, ReleaseError> {
    let receipt: Receipt = read_json(path, "build receipt")?;
    if receipt.target != target {
        return Err(ReleaseError::ReceiptUnpinned {
            target: target.to_owned(),
            field: "target".to_owned(),
            pinned: target.to_owned(),
            found: receipt.target,
        });
    }
    if receipt.host != Platform::of_triple(target)? {
        return Err(ReleaseError::CrossBuildRefused {
            target: target.to_owned(),
            host: receipt.host.label(),
        });
    }
    Ok(receipt)
}

fn read_text(path: &Path) -> Result<String, ReleaseError> {
    std::fs::read_to_string(path).map_err(ReleaseError::io("read", path))
}

fn versions(
    input: &StageInput,
    found: &Identity,
) -> Result<BTreeMap<String, String>, ReleaseError> {
    let schema: Value = serde_json::from_slice(&found.schema).unwrap_or(Value::Null);
    let package = input
        .repo
        .join("packages")
        .join("client-ts")
        .join("package.json");
    let client: Value = read_json(&package, "client-ts package.json")?;
    let sdk_name = input
        .sdk
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let sdk = sdk_name
        .strip_prefix("ovis-terminal-client-")
        .and_then(|rest| rest.strip_suffix(".tgz"));
    let text = |value: Option<&Value>| value.and_then(Value::as_str).unwrap_or_default().to_owned();
    let parts = BTreeMap::from([
        ("binary".to_owned(), found.version.clone()),
        ("manual".to_owned(), found.manual_version.clone()),
        ("schema".to_owned(), text(schema.pointer("/info/version"))),
        ("client_package".to_owned(), text(client.get("version"))),
        (
            "client_tarball".to_owned(),
            sdk.unwrap_or_default().to_owned(),
        ),
    ]);
    if let Some((part, value)) = parts.iter().find(|(_, value)| **value != found.version) {
        return Err(ReleaseError::VersionMismatch {
            part: match part.as_str() {
                "manual" => "the --skill manual",
                "schema" => "the OpenRPC info.version",
                "client_package" => "packages/client-ts/package.json",
                _ => "the SDK tarball name",
            },
            expected: found.version.clone(),
            found: value.clone(),
        });
    }
    Ok(parts)
}

/// Checks everything, then writes `<out>/<name>/` and `<out>/<name>.entry.json`.
///
/// # Errors
///
/// The first refused check: receipt, notices, versions, schema, then compatibility.
pub(crate) fn stage(input: &StageInput) -> Result<Value, ReleaseError> {
    let receipt = native_receipt(&input.receipt, &input.target)?;
    let lock = read_text(&input.repo.join("Cargo.lock"))?;
    let notice_crates = notices::check(&read_text(&input.notices())?, &lock)?;
    let found = identity(&input.binary)?;
    let versions = versions(input, &found)?;
    let source_schema = input.repo.join("schema").join("ovis-terminal.openrpc.json");
    if std::fs::read(&source_schema).map_err(ReleaseError::io("read", &source_schema))?
        != found.schema
    {
        return Err(ReleaseError::SchemaMismatch);
    }
    let compat = compat(&input.binary, &input.released())?;
    let checks = Checks {
        schema_matches_source: true,
        versions,
        compat,
        notice_crates: notice_crates.to_string(),
        apache_attribution: true,
    };
    let entry = lay_out(input, &found, (checks, receipt))?;
    Ok(json!({
        "staged": entry.name,
        "target": entry.target,
        "release_target": entry.release_target,
        "files": entry.files.len().to_string(),
    }))
}

fn lay_out(
    input: &StageInput,
    found: &Identity,
    (checks, receipt): (Checks, Receipt),
) -> Result<Entry, ReleaseError> {
    let name = format!("ovis-terminal-{}-{}", found.version, input.target);
    let dir = input.out.join(&name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(ReleaseError::io("replace", &dir))?;
    }
    std::fs::create_dir_all(&dir).map_err(ReleaseError::io("create", &dir))?;
    let sdk_name = input.sdk.file_name().map(PathBuf::from).unwrap_or_default();
    copy(&input.binary, &dir.join(binary_name(&input.target)))?;
    copy(&input.sdk, &dir.join(&sdk_name))?;
    copy(&input.repo.join("LICENSE"), &dir.join("LICENSE"))?;
    copy(&input.notices(), &dir.join("THIRD_PARTY_NOTICES.md"))?;
    let write = |file: &str, bytes: &[u8]| {
        let path = dir.join(file);
        std::fs::write(&path, bytes).map_err(ReleaseError::io("write", &path))
    };
    write("ovis-terminal.openrpc.json", &found.schema)?;
    write("manual.md", found.manual.as_bytes())?;
    write_json(&dir.join("receipt.json"), &receipt)?;
    let entry = Entry {
        format: "ovis-release-entry/1".to_owned(),
        name: name.clone(),
        version: found.version.clone(),
        generation: found.generation.clone(),
        target: input.target.clone(),
        release_target: is_release_target(&input.target),
        files: records(&dir)?,
        checks,
        receipt,
        archive: None,
    };
    write_json(&input.out.join(format!("{name}.entry.json")), &entry)?;
    Ok(entry)
}

/// Every file of `dir`, sorted by name, with its digest.
///
/// # Errors
///
/// [`ReleaseError::Io`] when the directory or a file cannot be read.
pub(crate) fn records(dir: &Path) -> Result<Vec<FileRecord>, ReleaseError> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map_err(ReleaseError::io("list", dir))?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()
        .map_err(ReleaseError::io("list", dir))?;
    names.sort();
    names
        .into_iter()
        .map(|path| {
            let (sha256, bytes) = sha256_file(&dir.join(&path))?;
            Ok(FileRecord {
                path,
                sha256,
                bytes,
            })
        })
        .collect()
}

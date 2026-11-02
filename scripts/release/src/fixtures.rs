//! Seeded conformance fixtures (`release-stage fixture <name>`).
//!
//! The native ones run the real binary under test; the promotion ones use the synthetic five-target matrix. Each
//! returns the exit status and report the step it exercises produced.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::binary::{compat, identity};
use crate::docs::{Entry, Receipt};
use crate::error::ReleaseError;
use crate::files::{read_json, sha256_file, write_json};
use crate::seal::{seal, verify};
use crate::stage::{StageInput, stage};
use crate::target::{Platform, binary_name};

/// Where a fixture runs: the binary under test, the source tree and a private scratch root.
#[derive(Debug, Clone)]
pub(crate) struct Paths {
    pub(crate) binary: PathBuf,
    pub(crate) repo: PathBuf,
    pub(crate) scratch: PathBuf,
}

/// An output this build lacks, added as required to a released result.
const REMOVED_OUTPUT: &str = "release_fixture_required_output";

pub(crate) fn outcome(result: Result<Value, ReleaseError>) -> (u8, Value) {
    result.map_or_else(
        |error| (error.exit(), crate::failure(&error)),
        |report| (0, report),
    )
}

fn fresh(dir: &Path) -> Result<(), ReleaseError> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(ReleaseError::io("clear", dir))?;
    }
    std::fs::create_dir_all(dir).map_err(ReleaseError::io("create", dir))
}

fn host_triple() -> String {
    let host = Platform::host();
    match host.os.as_str() {
        "macos" => format!("{}-apple-darwin", host.arch),
        "windows" => format!("{}-pc-windows-msvc", host.arch),
        _ => format!("{}-unknown-linux-gnu", host.arch),
    }
}

/// A stage input for the binary under test: from its own build receipt and packed SDK when
/// it was unpacked from a `build.sh` artifact, otherwise from fixture stand-ins.
fn native_input(paths: &Paths, work: &Path) -> Result<StageInput, ReleaseError> {
    if let Some(built) = crate::build_output::beside(&paths.binary)? {
        return Ok(StageInput {
            target: built.target,
            binary: paths.binary.clone(),
            sdk: built.sdk,
            receipt: built.receipt,
            repo: paths.repo.clone(),
            out: work.join("out"),
            released: None,
            notices: None,
        });
    }
    let version = identity(&paths.binary)?.version;
    let target = host_triple();
    let receipt = Receipt {
        format: "ovis-release-receipt/1".to_owned(),
        target: target.clone(),
        host: Platform::host(),
        runner: "conformance-fixture".to_owned(),
        rustc: "fixture".to_owned(),
        sqlite: "fixture".to_owned(),
        libsqlite3_sys: "fixture".to_owned(),
        c_toolchain: [("compiler".to_owned(), "fixture".to_owned())].into(),
    };
    write_json(&work.join("receipt.json"), &receipt)?;
    let sdk = work.join(format!("ovis-terminal-client-{version}.tgz"));
    std::fs::write(&sdk, b"fixture SDK bytes\n").map_err(ReleaseError::io("write", &sdk))?;
    Ok(StageInput {
        target,
        binary: paths.binary.clone(),
        sdk,
        receipt: work.join("receipt.json"),
        repo: paths.repo.clone(),
        out: work.join("out"),
        released: None,
        notices: None,
    })
}

fn native_artifact(paths: &Paths, work: &Path) -> Result<Value, ReleaseError> {
    let input = native_input(paths, work)?;
    let staged = stage(&input)?;
    let name = staged
        .get("staged")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let entry_path = input.out.join(format!("{name}.entry.json"));
    let sealed = seal(&entry_path)?;
    let qa = crate::qa::qa(&crate::qa::QaInput {
        entry: entry_path.clone(),
        install_dir: work.join("install"),
        released: paths.repo.join("schema").join("released.openrpc.json"),
        runner: "conformance-fixture".to_owned(),
    })?;
    let verified = verify(&input.out)?;
    let entry: Entry = read_json(&entry_path, "release entry")?;
    let observed = observe(paths, (&input, &entry))?;
    let receipt: Value = read_json(
        &input.out.join(format!("{name}.qa.json")),
        "native QA receipt",
    )?;
    Ok(json!({
        "staged": staged, "sealed": sealed, "qa": qa, "qa_receipt": receipt,
        "verified": verified, "entry": entry, "observed": observed,
        "inputs": if entry.receipt.runner == "conformance-fixture" { "fixture" } else { "build_output" },
    }))
}

/// Facts read from bytes, not from the entry: the binary under test, the staged and the
/// unpacked binary, the archive and its sidecar, and the source schema.
fn observe(paths: &Paths, (input, entry): (&StageInput, &Entry)) -> Result<Value, ReleaseError> {
    let binary = binary_name(&entry.target);
    let recorded = |path: &str| {
        entry
            .files
            .iter()
            .find(|file| file.path == path)
            .map(|file| file.sha256.clone())
    };
    let installed = work_install(input).join(&entry.name).join(binary);
    let found = identity(&installed)?;
    let archive = entry
        .archive
        .clone()
        .ok_or_else(|| ReleaseError::Unsealed {
            name: entry.name.clone(),
        })?;
    let sidecar_path = input.out.join(format!("{}.sha256", archive.file));
    let sidecar =
        std::fs::read_to_string(&sidecar_path).map_err(ReleaseError::io("read", &sidecar_path))?;
    let source_schema = paths.repo.join("schema").join("ovis-terminal.openrpc.json");
    let under_test = sha256_file(&paths.binary)?.0;
    let installed_sha = sha256_file(&installed)?.0;
    let expected_schema = sha256_file(&source_schema)?.0;
    let installed_schema = crate::files::sha256_bytes(&found.schema);
    let archive_bytes = sha256_file(&input.out.join(&archive.file))?.0;
    let sidecar_sha = sidecar
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned();
    let staged = recorded(binary);
    Ok(json!({
        "binary_under_test_sha256": under_test,
        "staged_binary_sha256": staged,
        "installed_binary_sha256": installed_sha,
        "installed_version": found.version,
        "expected_schema_sha256": expected_schema,
        "installed_schema_sha256": installed_schema,
        "archive_actual_sha256": archive_bytes,
        "sidecar_sha256": sidecar_sha,
        "agree": {
            "staged_binary_is_binary_under_test": staged.as_deref() == Some(under_test.as_str()),
            "installed_binary_is_binary_under_test": installed_sha == under_test,
            "installed_version_is_entry_version": found.version == entry.version,
            "installed_schema_is_source_schema": installed_schema == expected_schema,
            "archive_bytes_match_entry": archive_bytes == archive.sha256,
            "sidecar_matches_archive_bytes": sidecar_sha == archive_bytes,
        },
    }))
}

fn work_install(input: &StageInput) -> PathBuf {
    input
        .out
        .parent()
        .map_or_else(|| PathBuf::from("install"), |work| work.join("install"))
}

fn removed_required_output(paths: &Paths, work: &Path) -> Result<(u8, Value), ReleaseError> {
    let baseline = paths.repo.join("schema").join("released.openrpc.json");
    let (before, _) = sha256_file(&baseline)?;
    let mut released: Value = read_json(&baseline, "released OpenRPC document")?;
    let result = released
        .pointer_mut("/components/schemas/ServerStatusResult")
        .and_then(Value::as_object_mut);
    if let Some(result) = result {
        result.entry("required").or_insert_with(|| json!([]));
        if let Some(required) = result.get_mut("required").and_then(Value::as_array_mut) {
            required.push(json!(REMOVED_OUTPUT));
        }
        if let Some(properties) = result.get_mut("properties").and_then(Value::as_object_mut) {
            properties.insert(REMOVED_OUTPUT.to_owned(), json!({ "type": "string" }));
        }
    }
    let mutated = work.join("released.openrpc.json");
    write_json(&mutated, &released)?;
    let mut input = native_input(paths, work)?;
    input.released = Some(mutated);
    let (exit, mut report) = outcome(stage(&input));
    let (after, _) = sha256_file(&baseline)?;
    let entries = crate::seal::entries(&input.out).map_or(0, |found| found.len());
    if let Some(fields) = report.as_object_mut() {
        fields.insert("released_unchanged".to_owned(), json!(before == after));
        fields.insert("released_sha256".to_owned(), json!(after));
        fields.insert("entries_written".to_owned(), json!(entries.to_string()));
        fields.insert("removed_output".to_owned(), json!(REMOVED_OUTPUT));
    }
    Ok((exit, report))
}

fn missing_notices(paths: &Paths, work: &Path) -> Result<Value, ReleaseError> {
    let source = paths.repo.join("THIRD_PARTY_NOTICES.md");
    let text = std::fs::read_to_string(&source).map_err(ReleaseError::io("read", &source))?;
    let stripped = work.join("THIRD_PARTY_NOTICES.md");
    let without = text.replace(
        "## herdr and herdr-core (Apache-2.0)",
        "## herdr and herdr-core",
    );
    std::fs::write(&stripped, without).map_err(ReleaseError::io("write", &stripped))?;
    let mut input = native_input(paths, work)?;
    input.notices = Some(stripped);
    stage(&input)
}

/// Runs fixture `name` in `<scratch>/release-<name>`.
pub(crate) fn run(name: &str, paths: &Paths) -> Result<(u8, Value), ReleaseError> {
    let work = paths.scratch.join(format!("release-{name}"));
    fresh(&work)?;
    let (exit, mut report) = match name {
        "released-baseline" => {
            let released = paths.repo.join("schema").join("released.openrpc.json");
            outcome(
                compat(&paths.binary, &released)
                    .map(|binary| json!({ "compatible": true, "binary": binary })),
            )
        }
        "native-artifact" => outcome(native_artifact(paths, &work)),
        "same-generation-removed-required-output" => removed_required_output(paths, &work)?,
        "missing-notices" => outcome(missing_notices(paths, &work)),
        "complete-matrix-dry-run"
        | "missing-windows-artifact"
        | "missing-native-qa"
        | "cross-built-macos"
        | "unpinned-sqlite"
        | "bad-manifest-hash"
        | "malformed-manifest"
        | "rerun-after-publish"
        | "conflicting-rerun"
        | "wrong-sidecar"
        | "failed-native-qa"
        | "forged-qa-pass"
        | "repacked-archive" => crate::gate_fixtures::matrix_fixture(name, paths, &work)?,
        _ => {
            return Err(ReleaseError::UnknownFixture {
                name: name.to_owned(),
            });
        }
    };
    if let Some(fields) = report.as_object_mut() {
        fields.insert("fixture".to_owned(), json!(name));
    }
    Ok((exit, report))
}

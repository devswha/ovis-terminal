//! One probe run end to end: fixture releases, the release server, every selected case
//! and the report that carries their evidence.

use std::path::Path;

use serde_json::{Value, json};

use super::archive::sha256_file;
use super::cases::{Context, run_case};
use super::error::InstallProbeError;
use super::fixture::{Fixture, Inputs, MISLABELED_VERSION, build};
use super::probe::Expected;
use super::serve::Server;

/// Where a run reads and writes.
pub(super) struct Setup<'a> {
    pub(super) cases: &'a [&'static str],
    pub(super) checkout: &'a Path,
    pub(super) binary: &'a Path,
    pub(super) repo: &'a Path,
    pub(super) scratch: &'a Path,
}

fn binary_version(binary: &Path) -> Result<String, InstallProbeError> {
    let output = std::process::Command::new(binary)
        .arg("--version")
        .output()
        .map_err(|source| InstallProbeError::Spawn {
            program: binary.display().to_string(),
            source,
        })?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text.trim().trim_start_matches("ovis-terminal ").to_owned())
}

/// The environment every case command sees besides its private directories.
fn urls(server: &Server, fixture: &Fixture) -> Vec<(String, String)> {
    [
        ("FIXTURE_RELEASES_URL", server.url("good")),
        ("CORRUPT_RELEASES_URL", server.url("corrupt")),
        ("TRAVERSAL_RELEASES_URL", server.url("traversal")),
        ("UNKNOWN_TARGET_RELEASES_URL", server.url("unknown-target")),
        ("MISLABELED_RELEASES_URL", server.url("mislabeled")),
        ("STALL_RELEASES_URL", server.url("stall/good")),
        ("FIXTURE_VERSION", fixture.version.clone()),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value))
    .collect()
}

/// Builds the fixtures, serves them and runs every selected case.
pub(super) fn cases(setup: &Setup<'_>) -> Result<(u8, Value), InstallProbeError> {
    let fixture = build(&Inputs {
        binary: setup.binary,
        repo: setup.repo,
        scratch: setup.scratch,
    })?;
    let binary_sha256 = sha256_file(setup.binary)?;
    let version = binary_version(setup.binary)?;
    let server = Server::start(&fixture.root)?;
    let urls = urls(&server, &fixture);
    let expected = Expected {
        binary_sha256: &binary_sha256,
        version: &version,
    };
    let ctx = Context {
        scratch: setup.scratch,
        checkout: setup.checkout,
        urls: &urls,
        server: &server,
        expected: &expected,
    };
    let results = setup
        .cases
        .iter()
        .map(|case| run_case(&ctx, case))
        .collect::<Result<Vec<Value>, _>>()?;
    let failed = results
        .iter()
        .filter(|case| case.get("passed") != Some(&json!(true)))
        .count();
    let exit = u8::from(failed > 0);
    Ok((
        exit,
        json!({
            "verdict": if failed == 0 { "pass" } else { "fail" },
            "failed": failed.to_string(),
            "cases_run": results.len().to_string(),
            "host": { "os": std::env::consts::OS, "arch": std::env::consts::ARCH },
            "binary": { "path": setup.binary, "sha256": binary_sha256, "version": version },
            "script": {
                "path": setup.checkout.join("install.sh"),
                "sha256": sha256_file(&setup.checkout.join("install.sh"))?,
            },
            "fixture": {
                "version": fixture.version,
                "archive": fixture.archive,
                "archive_sha256": fixture.archive_sha256,
                "mislabeled_version": MISLABELED_VERSION,
                "env": urls.iter().map(|(name, value)| json!({ "name": name, "value": value })).collect::<Vec<_>>(),
            },
            "cases": results,
        }),
    ))
}

//! `receipt`: what the build host and toolchain are, observed rather than declared.
//!
//! The tool reads rustc and the bundled SQLite itself; the build script passes the C toolchain
//! observations (`--observe name=value`) it read from the compiler, SDK or toolset.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::docs::Receipt;
use crate::error::ReleaseError;
use crate::files::helper;
use crate::target::Platform;

/// Inputs of one receipt.
#[derive(Debug, Clone)]
pub(crate) struct ReceiptInput {
    /// The target triple being built.
    pub(crate) target: String,
    /// The runner label, or `local`.
    pub(crate) runner: String,
    /// The source tree.
    pub(crate) repo: PathBuf,
    /// `name=value` C toolchain observations.
    pub(crate) observed: Vec<String>,
}

/// The value of `key: value` in `rustc -vV` output.
fn rustc_field(verbose: &str, key: &str) -> Option<String> {
    verbose.lines().find_map(|line| {
        line.strip_prefix(key)?
            .strip_prefix(": ")
            .map(str::to_owned)
    })
}

/// The `#define SQLITE_VERSION "x.y.z"` of a bundled `sqlite3.h`.
#[must_use]
pub(crate) fn sqlite_version(header: &str) -> Option<String> {
    header.lines().find_map(|line| {
        let rest = line.strip_prefix("#define SQLITE_VERSION")?;
        let quoted = rest.trim();
        quoted
            .strip_prefix('"')?
            .strip_suffix('"')
            .map(str::to_owned)
    })
}

fn bundled_sqlite(repo: &Path) -> Result<(String, String), ReleaseError> {
    let stdout = helper(
        Command::new("cargo")
            .args([
                "metadata",
                "--locked",
                "--format-version",
                "1",
                "--manifest-path",
            ])
            .arg(repo.join("Cargo.toml")),
    )?;
    let missing = |detail: &str| ReleaseError::Tool {
        program: "cargo metadata".to_owned(),
        detail: detail.to_owned(),
    };
    let metadata: Value =
        serde_json::from_slice(&stdout).map_err(|_| missing("output is not JSON"))?;
    let package = metadata
        .get("packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|package| package.get("name") == Some(&Value::from("libsqlite3-sys")))
        .ok_or_else(|| missing("no libsqlite3-sys package"))?;
    let version = package
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let manifest = PathBuf::from(
        package
            .get("manifest_path")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    let header = manifest.with_file_name("sqlite3").join("sqlite3.h");
    let text = std::fs::read_to_string(&header).map_err(ReleaseError::io("read", &header))?;
    let sqlite = sqlite_version(&text).ok_or_else(|| missing("sqlite3.h has no SQLITE_VERSION"))?;
    Ok((sqlite, version))
}

/// Observes the build and returns its receipt.
///
/// # Errors
///
/// A malformed observation, an unknown target, or `rustc`/`cargo` failures.
pub(crate) fn receipt(input: &ReceiptInput) -> Result<Receipt, ReleaseError> {
    Platform::of_triple(&input.target)?;
    let mut c_toolchain = BTreeMap::new();
    for observation in &input.observed {
        let (name, value) = observation
            .split_once('=')
            .ok_or_else(|| ReleaseError::Usage {
                detail: format!("--observe {observation} is not name=value"),
            })?;
        c_toolchain.insert(name.to_owned(), value.trim().to_owned());
    }
    let verbose = String::from_utf8_lossy(&helper(
        Command::new("rustc").arg("-vV").current_dir(&input.repo),
    )?)
    .into_owned();
    let (sqlite, libsqlite3_sys) = bundled_sqlite(&input.repo)?;
    Ok(Receipt {
        format: "ovis-release-receipt/1".to_owned(),
        target: input.target.clone(),
        host: Platform::host(),
        runner: input.runner.clone(),
        rustc: rustc_field(&verbose, "release").unwrap_or_default(),
        sqlite,
        libsqlite3_sys,
        c_toolchain,
    })
}

#[cfg(test)]
mod tests {
    use super::{rustc_field, sqlite_version};

    #[test]
    fn the_bundled_sqlite_version_comes_from_its_header_define() {
        let header =
            "#define SQLITE_VERSION_NUMBER 3054001\n#define SQLITE_VERSION        \"3.54.1\"\n";
        assert_eq!(sqlite_version(header).as_deref(), Some("3.54.1"));
        assert_eq!(sqlite_version("#define SQLITE_VERSION_NUMBER 1\n"), None);
    }

    #[test]
    fn the_rustc_release_is_read_from_its_verbose_version() {
        let verbose = "rustc 1.99.1 (58cf8f9c3 2026-10-13)\nhost: x86_64-unknown-linux-gnu\nrelease: 1.99.1\n";
        assert_eq!(rustc_field(verbose, "release").as_deref(), Some("1.99.1"));
        assert_eq!(
            rustc_field(verbose, "host").as_deref(),
            Some("x86_64-unknown-linux-gnu")
        );
    }
}

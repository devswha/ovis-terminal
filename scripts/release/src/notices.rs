//! `THIRD_PARTY_NOTICES.md`: the Apache-2.0 attribution for mirrored herdr material and the
//! license inventory of every registry crate in Cargo.lock.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::error::ReleaseError;
use crate::files::helper;

const BEGIN: &str = "<!-- crate-inventory:begin -->";
const END: &str = "<!-- crate-inventory:end -->";

/// Text the attribution needs, and what each missing piece is called.
const REQUIRED: [(&str, &str); 3] = [
    (
        "## herdr and herdr-core (Apache-2.0)",
        "the herdr/herdr-core Apache-2.0 attribution",
    ),
    (
        "Licensed under the Apache License, Version 2.0",
        "the Apache-2.0 notice for mirrored material",
    ),
    (
        "TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION",
        "the Apache-2.0 license text",
    ),
];

fn field<'a>(block: &'a str, key: &str) -> Option<&'a str> {
    block.lines().find_map(|line| {
        line.strip_prefix(key)
            .and_then(|rest| rest.strip_prefix(" = \""))
            .and_then(|rest| rest.strip_suffix('"'))
    })
}

/// `(name, version)` of every crates.io package in a Cargo.lock.
#[must_use]
pub(crate) fn registry_crates(lock: &str) -> Vec<(String, String)> {
    lock.split("[[package]]")
        .filter(|block| {
            field(block, "source").is_some_and(|source| source.starts_with("registry+"))
        })
        .filter_map(|block| {
            Some((
                field(block, "name")?.to_owned(),
                field(block, "version")?.to_owned(),
            ))
        })
        .collect()
}

/// Checks the attribution and that the inventory lists every registry crate of `lock`;
/// returns how many crates it covers.
///
/// # Errors
///
/// [`ReleaseError::NoticesMissing`] or [`ReleaseError::NoticesIncomplete`].
pub(crate) fn check(notices: &str, lock: &str) -> Result<usize, ReleaseError> {
    if let Some((_, missing)) = REQUIRED
        .iter()
        .find(|(needle, _)| !notices.contains(needle))
    {
        return Err(ReleaseError::NoticesMissing { missing });
    }
    let inventory = section(notices).ok_or(ReleaseError::NoticesMissing {
        missing: "the crate license inventory",
    })?;
    let crates = registry_crates(lock);
    let unlisted: Vec<String> = crates
        .iter()
        .filter(|(name, version)| !inventory.contains(&format!("\n| {name} | {version} |")))
        .map(|(name, version)| format!("{name} {version}"))
        .collect();
    if unlisted.is_empty() {
        Ok(crates.len())
    } else {
        Err(ReleaseError::NoticesIncomplete { crates: unlisted })
    }
}

fn section(notices: &str) -> Option<&str> {
    let start = notices.find(BEGIN)?;
    let end = notices.find(END)?;
    notices.get(start..end)
}

fn cell(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or("-")
        .replace('|', "/")
}

/// The inventory table for `cargo metadata` output: registry packages, sorted.
#[must_use]
pub(crate) fn inventory(metadata: &Value) -> String {
    let mut rows: Vec<String> = metadata
        .get("packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|package| {
            package
                .get("source")
                .and_then(Value::as_str)
                .is_some_and(|s| s.starts_with("registry+"))
        })
        .map(|package| {
            let license = package
                .get("license")
                .filter(|value| value.is_string())
                .map_or_else(
                    || format!("see {}", cell(package.get("license_file"))),
                    |value| cell(Some(value)),
                );
            format!(
                "| {} | {} | {license} | {} |",
                cell(package.get("name")),
                cell(package.get("version")),
                cell(package.get("repository"))
            )
        })
        .collect();
    rows.sort();
    let mut table = String::from("| Crate | Version | License | Repository |\n|---|---|---|---|\n");
    for row in rows {
        table.push_str(&row);
        table.push('\n');
    }
    table
}

/// `notices` replaced between the inventory markers by `table`.
#[must_use]
pub(crate) fn with_inventory(notices: &str, table: &str) -> Option<String> {
    let start = notices.find(BEGIN)?.checked_add(BEGIN.len())?;
    let end = notices.find(END)?;
    Some(format!(
        "{}\n{table}{}",
        notices.get(..start)?,
        notices.get(end..)?
    ))
}

/// Regenerates (or with `check`, verifies) the inventory of `<repo>/THIRD_PARTY_NOTICES.md`.
///
/// # Errors
///
/// [`ReleaseError::NoticesStale`] when checking finds a difference, or the failures of
/// `cargo metadata` and the file access.
pub(crate) fn refresh(repo: &Path, check_only: bool) -> Result<usize, ReleaseError> {
    let manifest = repo.join("Cargo.toml");
    let stdout = helper(
        Command::new("cargo")
            .args([
                "metadata",
                "--locked",
                "--format-version",
                "1",
                "--manifest-path",
            ])
            .arg(&manifest),
    )?;
    let metadata: Value =
        serde_json::from_slice(&stdout).map_err(|source| ReleaseError::Malformed {
            path: manifest.clone(),
            document: "cargo metadata report",
            source,
        })?;
    let path = repo.join("THIRD_PARTY_NOTICES.md");
    let current = std::fs::read_to_string(&path).map_err(ReleaseError::io("read", &path))?;
    let table = inventory(&metadata);
    let updated = with_inventory(&current, &table).ok_or(ReleaseError::NoticesMissing {
        missing: "the crate-inventory markers",
    })?;
    let rows = table.lines().count().saturating_sub(2);
    if updated == current {
        return Ok(rows);
    }
    if check_only {
        return Err(ReleaseError::NoticesStale);
    }
    std::fs::write(&path, updated).map_err(ReleaseError::io("write", &path))?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{BEGIN, END, REQUIRED, check, inventory, registry_crates, with_inventory};

    const LOCK: &str = "[[package]]\nname = \"ours\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n";

    fn notices(rows: &str) -> String {
        let attribution = REQUIRED.map(|(needle, _)| needle).join("\n");
        format!("{attribution}\n{BEGIN}\n| Crate | Version | License | Repository |\n{rows}{END}\n")
    }

    #[test]
    fn only_registry_packages_need_an_inventory_row() {
        assert_eq!(
            registry_crates(LOCK),
            [("serde".to_owned(), "1.0.0".to_owned())]
        );
        assert_eq!(
            check(&notices("| serde | 1.0.0 | MIT | - |\n"), LOCK).ok(),
            Some(1)
        );
    }

    #[test]
    fn an_unlisted_crate_or_a_lost_attribution_fails_with_its_own_kind() {
        let missing_row = check(&notices("| serde | 0.9.0 | MIT | - |\n"), LOCK);
        assert_eq!(
            missing_row.map_err(|error| error.kind()),
            Err("notices_incomplete")
        );
        let no_apache = notices("| serde | 1.0.0 | MIT | - |\n").replace("Version 2.0", "");
        assert_eq!(
            check(&no_apache, LOCK).map_err(|error| error.kind()),
            Err("notices_missing")
        );
    }

    #[test]
    fn the_generated_inventory_is_sorted_and_names_a_license_file_when_no_expression() {
        let metadata = json!({"packages": [
            {"name": "zeta", "version": "1.0.0", "source": "registry+x", "license": "MIT", "repository": null},
            {"name": "alpha", "version": "2.0.0", "source": "registry+x", "license": null, "license_file": "LICENSE"},
            {"name": "ours", "version": "0.1.0", "source": null, "license": "MIT"}
        ]});
        let table = inventory(&metadata);
        let rows: Vec<&str> = table.lines().skip(2).collect();
        assert_eq!(
            rows,
            [
                "| alpha | 2.0.0 | see LICENSE | - |",
                "| zeta | 1.0.0 | MIT | - |"
            ]
        );
        let replaced =
            with_inventory(&notices("| stale | 0 | - | - |\n"), &table).unwrap_or_default();
        assert!(!replaced.contains("stale") && replaced.contains("| zeta | 1.0.0 |"));
    }
}

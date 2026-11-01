//! The strict Cargo configuration: workspace lint table, member inheritance, toolchain
//! pin, cargo-deny config and the clippy thresholds (V-001, V-002, V-3895, V-3896).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::error::QualityError;
use super::report::Finding;
use super::scan::shown;

/// Every rule id the Cargo configuration check can report.
pub(super) const CARGO_RULES: &[&str] = &[
    "workspace-lints",
    "member-lints",
    "toolchain-pinned",
    "cargo-deny-config",
    "clippy-thresholds",
];

/// Lints the binding contract requires at `deny` (V-3896 lists what herdr-core lacked).
const DENIED_CLIPPY: &[&str] = &[
    "all",
    "unwrap_used",
    "expect_used",
    "panic",
    "todo",
    "unimplemented",
    "indexing_slicing",
    "let_underscore_must_use",
    "allow_attributes",
    "allow_attributes_without_reason",
    "undocumented_unsafe_blocks",
];
const DENIED_RUST: &[&str] = &["unused_must_use", "unsafe_op_in_unsafe_fn"];
const ENABLED_GROUPS: &[&str] = &["pedantic", "nursery"];
/// clippy.toml keys and the loosest value the contract accepts.
const THRESHOLDS: &[(&str, u32)] = &[
    ("too-many-arguments-threshold", 3),
    ("too-many-lines-threshold", 100),
];
const SKIPPED_DIRS: &[&str] = &["target", ".git", "node_modules", ".omo", "herdr-core"];

/// `name -> level` of one `[table]` of a TOML manifest, read line by line.
pub(super) fn lint_table(manifest: &str, table: &str) -> BTreeMap<String, String> {
    let header = format!("[{table}]");
    let mut inside = false;
    let mut levels = BTreeMap::new();
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == header;
            continue;
        }
        let Some((name, value)) = line.split_once('=').filter(|_| inside) else {
            continue;
        };
        let level = ["deny", "warn", "allow", "forbid"]
            .into_iter()
            .find(|level| value.contains(&format!("\"{level}\"")));
        if let Some(level) = level {
            levels.insert(name.trim().to_owned(), level.to_owned());
        }
    }
    levels
}

fn read(path: &Path) -> Result<String, QualityError> {
    std::fs::read_to_string(path).map_err(QualityError::io(path))
}

fn require_level(
    levels: &BTreeMap<String, String>,
    wanted: (&str, &[&str]),
    out: &mut Vec<Finding>,
) {
    let (table, names) = wanted;
    for name in names {
        let level = levels.get(*name).map_or("absent", String::as_str);
        let enough = level == "deny"
            || level == "forbid"
            || (ENABLED_GROUPS.contains(name) && level == "warn");
        if !enough {
            let detail = format!("[{table}] {name} is {level}");
            out.push(Finding::new("workspace-lints", "Cargo.toml", detail));
        }
    }
}

/// Findings for the workspace manifest, the toolchain pin, deny.toml and clippy.toml.
pub(super) fn check_workspace(repo: &Path) -> Result<Vec<Finding>, QualityError> {
    let mut found = Vec::new();
    let manifest = read(&repo.join("Cargo.toml"))?;
    let clippy = lint_table(&manifest, "workspace.lints.clippy");
    let rust = lint_table(&manifest, "workspace.lints.rust");
    require_level(
        &clippy,
        ("workspace.lints.clippy", DENIED_CLIPPY),
        &mut found,
    );
    require_level(
        &clippy,
        ("workspace.lints.clippy", ENABLED_GROUPS),
        &mut found,
    );
    require_level(&rust, ("workspace.lints.rust", DENIED_RUST), &mut found);
    let pinned = std::fs::read_to_string(repo.join("rust-toolchain.toml")).is_ok_and(|text| {
        text.lines()
            .any(|line| line.trim_start().starts_with("channel"))
    });
    if !pinned {
        let detail = "rust-toolchain.toml with a channel is missing";
        found.push(Finding::new(
            "toolchain-pinned",
            "rust-toolchain.toml",
            detail,
        ));
    }
    if !repo.join("deny.toml").is_file() {
        found.push(Finding::new(
            "cargo-deny-config",
            "deny.toml",
            "deny.toml is missing",
        ));
    }
    let thresholds = read(&repo.join("clippy.toml"))?;
    for (key, loosest) in THRESHOLDS {
        let value = thresholds.lines().find_map(|line| {
            let (name, value) = line.split_once('=')?;
            (name.trim() == *key).then(|| value.trim().parse::<u32>().ok())?
        });
        if value.is_none_or(|value| value > *loosest) {
            let detail = format!("{key} must be set to at most {loosest}");
            found.push(Finding::new("clippy-thresholds", "clippy.toml", detail));
        }
    }
    Ok(found)
}

fn manifests(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), QualityError> {
    for entry in std::fs::read_dir(dir).map_err(QualityError::io(dir))? {
        let path = entry.map_err(QualityError::io(dir))?.path();
        let skipped = path
            .file_name()
            .is_some_and(|name| SKIPPED_DIRS.iter().any(|skip| name == *skip));
        if path.is_dir() && !skipped {
            manifests(&path, out)?;
        } else if path.file_name().is_some_and(|name| name == "Cargo.toml") {
            out.push(path);
        }
    }
    Ok(())
}

/// Every member manifest under the roots must inherit the workspace lints (V-001).
pub(super) fn check_members(repo: &Path, roots: &[PathBuf]) -> Result<Vec<Finding>, QualityError> {
    let mut paths = Vec::new();
    for root in roots {
        manifests(&repo.join(root), &mut paths)?;
    }
    paths.sort();
    let mut found = Vec::new();
    for path in paths {
        let text = read(&path)?;
        let inherits = lint_table(&text, "lints").is_empty()
            && text.lines().map(str::trim).collect::<Vec<_>>().windows(2).any(|pair| {
                matches!(pair, [header, body] if *header == "[lints]" && body.replace(' ', "") == "workspace=true")
            });
        let own_tables = text
            .lines()
            .any(|line| line.trim_start().starts_with("[lints."));
        if !inherits || own_tables {
            let detail = "member must declare `[lints] workspace = true` and no lint tables";
            found.push(Finding::new("member-lints", shown(repo, &path), detail));
        }
    }
    Ok(found)
}

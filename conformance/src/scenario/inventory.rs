//! Every file under the scenario root, classified: executed, declared only, or invalid.
//!
//! A file with `"format": "ovis-conformance/1"` that loads is executed. A JSON file with no
//! `format` key at all is a declaration and must be listed in `declared.json` with its owning
//! todo. Anything else (a misspelled marker, a name outside the id grammar, a non-JSON file,
//! a parse error) is invalid. Nothing is dropped: the suite reports all three lists.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::load::{Repo, load, read_json};
use super::model::{FORMAT, ScenarioFile};
use crate::error::RunnerError;
use crate::ids::ScenarioId;

/// The allowlist of declaration files, at the scenario root.
pub(crate) const ALLOWLIST: &str = "declared.json";
const ALLOWLIST_FORMAT: &str = "ovis-declared-scenarios/1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Allowlist {
    format: String,
    entries: Vec<DeclaredEntry>,
}

/// One declaration that cannot run yet, who owns making it executable, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeclaredEntry {
    pub(crate) file: String,
    pub(crate) owner_todo: String,
    pub(crate) reason: String,
}

/// A file that is neither executable nor a listed declaration, and why.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct InvalidFile {
    pub(crate) file: String,
    pub(crate) kind: &'static str,
    pub(crate) detail: String,
}

/// The classification of every file under the scenario root.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Inventory {
    pub(crate) executed: Vec<String>,
    pub(crate) declared_only: Vec<DeclaredEntry>,
    pub(crate) invalid: Vec<InvalidFile>,
    /// Declarations missing from the allowlist.
    pub(crate) unlisted: Vec<String>,
    /// Allowlist entries that no longer name a declaration with that owner.
    pub(crate) stale: Vec<String>,
}

impl Inventory {
    /// The first reason the inventory fails the run, if any.
    pub(crate) fn problem(&self) -> Option<RunnerError> {
        if let Some(invalid) = self.invalid.first() {
            return Some(RunnerError::ScenarioFileInvalid {
                file: invalid.file.clone(),
                detail: invalid.detail.clone(),
            });
        }
        if let Some(file) = self.unlisted.first() {
            return Some(RunnerError::DeclarationUnlisted { file: file.clone() });
        }
        self.stale
            .first()
            .map(|file| RunnerError::DeclarationStale { file: file.clone() })
    }
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), RunnerError> {
    let io = |source| RunnerError::Io {
        operation: "list the scenario root",
        source,
    };
    for entry in std::fs::read_dir(dir).map_err(io)? {
        let path = entry.map_err(io)?.path();
        if path.is_dir() {
            files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

/// What one file is: an executable scenario, a declaration with its todo, or invalid.
enum Class {
    Executed(Box<ScenarioFile>),
    Declared(Option<String>),
    Invalid(&'static str, String),
}

fn classify(repo: &Repo, relative: &str) -> Class {
    let invalid = |detail: String| Class::Invalid("scenario_invalid", detail);
    let Some(id) = relative
        .strip_suffix(".json")
        .and_then(|stem| ScenarioId::parse(stem).ok())
    else {
        return invalid(
            "the path is not <family>/<name>.json in the scenario id grammar".to_owned(),
        );
    };
    let value = match read_json(&repo.scenarios_dir().join(relative)) {
        Ok(value) => value,
        Err(error) => return invalid(error.to_string()),
    };
    match value.get("format") {
        None => Class::Declared(value.get("todo").map(|todo| match todo {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })),
        Some(marker) if marker.as_str() != Some(FORMAT) => {
            invalid(format!("the format marker {marker} is not \"{FORMAT}\""))
        }
        Some(_) => match load(repo, &id) {
            Ok(file) => Class::Executed(Box::new(file)),
            Err(error) => Class::Invalid(error.kind(), error.to_string()),
        },
    }
}

fn allowlist(repo: &Repo) -> Result<Vec<DeclaredEntry>, RunnerError> {
    let path = repo.scenarios_dir().join(ALLOWLIST);
    let list: Allowlist = serde_path_to_error::deserialize(read_json(&path)?).map_err(|error| {
        RunnerError::ScenarioInvalid {
            path: path.clone(),
            field: error.path().to_string(),
            source: error.into_inner(),
        }
    })?;
    if list.format != ALLOWLIST_FORMAT {
        return Err(RunnerError::ScenarioFileInvalid {
            file: ALLOWLIST.to_owned(),
            detail: format!("the allowlist format is not \"{ALLOWLIST_FORMAT}\""),
        });
    }
    Ok(list.entries)
}

/// Classifies every file and returns the executable non-suite scenarios to run.
pub(crate) fn take(repo: &Repo) -> Result<(Inventory, Vec<ScenarioFile>), RunnerError> {
    let mut paths = Vec::new();
    files(repo.scenarios_dir(), &mut paths)?;
    let mut inventory = Inventory::default();
    let mut runnable = Vec::new();
    let mut declared: BTreeMap<String, Option<String>> = BTreeMap::new();
    for path in paths {
        let relative = path.strip_prefix(repo.scenarios_dir()).map_or_else(
            |_| path.display().to_string(),
            |relative| relative.to_string_lossy().replace('\\', "/"),
        );
        if relative == ALLOWLIST {
            continue;
        }
        match classify(repo, &relative) {
            Class::Executed(file) => {
                inventory.executed.push(relative);
                if file.suite.is_none() {
                    runnable.push(*file);
                }
            }
            Class::Declared(todo) => {
                declared.insert(relative, todo);
            }
            Class::Invalid(kind, detail) => inventory.invalid.push(InvalidFile {
                file: relative,
                kind,
                detail,
            }),
        }
    }
    let listed = allowlist(repo)?;
    for entry in &listed {
        match declared.get(&entry.file) {
            Some(Some(todo)) if *todo == entry.owner_todo => {}
            Some(_) | None => inventory.stale.push(entry.file.clone()),
        }
    }
    for (file, todo) in declared {
        if let Some(entry) = listed.iter().find(|entry| entry.file == file) {
            inventory.declared_only.push(entry.clone());
        } else {
            inventory.declared_only.push(DeclaredEntry {
                file: file.clone(),
                owner_todo: todo.unwrap_or_default(),
                reason: "not in declared.json".to_owned(),
            });
            inventory.unlisted.push(file);
        }
    }
    inventory.executed.sort();
    inventory
        .invalid
        .sort_by(|left, right| left.file.cmp(&right.file));
    runnable.sort_by(|left, right| left.scenario.cmp(&right.scenario));
    Ok((inventory, runnable))
}

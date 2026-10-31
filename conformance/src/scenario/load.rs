//! Finding, parsing and planning scenarios; a declaration file is never treated as a test.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::model::{Case, FORMAT, Platform, ScenarioFile, Step};
use crate::error::RunnerError;
use crate::evidence::Declared;
use crate::ids::{CaseName, ScenarioId};

const MAX_INCLUDE_DEPTH: usize = 4;

/// The repository the runner was built from: scenarios, fixtures and published files.
#[derive(Debug, Clone)]
pub(crate) struct Repo {
    root: PathBuf,
}

impl Repo {
    /// The checkout containing this crate.
    pub(crate) fn of_build() -> Self {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        Self {
            root: manifest.parent().unwrap_or(manifest).to_path_buf(),
        }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn conformance(&self, relative: &str) -> PathBuf {
        self.root.join("conformance").join(relative)
    }

    /// Reads and parses a JSON file below `conformance/`.
    pub(crate) fn json(&self, relative: &str) -> Result<Value, RunnerError> {
        let path = self.conformance(relative);
        let bytes = std::fs::read(&path).map_err(|source| RunnerError::Io {
            operation: "read a conformance file",
            source,
        })?;
        serde_json::from_slice(&bytes).map_err(|source| RunnerError::ScenarioInvalid {
            path,
            field: ".".to_owned(),
            source,
        })
    }
}

/// One case ready to run, with includes already spliced in.
#[derive(Debug, Clone)]
pub(crate) struct PlannedCase {
    pub(crate) scenario: ScenarioId,
    pub(crate) case: CaseName,
    pub(crate) summary: String,
    pub(crate) declared: Declared,
    pub(crate) platforms: Vec<Platform>,
    pub(crate) steps: Vec<Step>,
}

/// Cases to run for `scenario`; `case` of `None` selects every declared case.
pub(crate) fn plan(
    repo: &Repo,
    scenario: &ScenarioId,
    case: Option<CaseName>,
) -> Result<Vec<PlannedCase>, RunnerError> {
    let file = load(repo, scenario)?;
    let members = if file.suite.is_some() {
        registered(repo)?
    } else {
        vec![file]
    };
    let mut planned = Vec::new();
    for member in members {
        let selected: Vec<&Case> = member
            .cases
            .iter()
            .filter(|declared| case.is_none_or(|wanted| declared.name == wanted))
            .collect();
        if selected.is_empty() {
            return Err(RunnerError::CaseMissing {
                scenario: member.scenario.to_string(),
                case: case.map_or("any", CaseName::as_str),
            });
        }
        let declares = Declared {
            title: member.title.clone(),
            todo: member.todo.to_string(),
            closes: member.closes.clone(),
            delivers: member.delivers.clone(),
            excludes: member.excludes.clone(),
        };
        for declared in selected {
            planned.push(PlannedCase {
                scenario: member.scenario.clone(),
                case: declared.name,
                summary: declared.summary.clone(),
                declared: declares.clone(),
                platforms: member.platforms.clone(),
                steps: expand(repo, &declared.steps, 0)?,
            });
        }
    }
    Ok(planned)
}

/// Every registered scenario except suites, ordered by identity.
pub(crate) fn registered(repo: &Repo) -> Result<Vec<ScenarioFile>, RunnerError> {
    let io = |source| RunnerError::Io {
        operation: "list conformance/scenarios",
        source,
    };
    let mut found = Vec::new();
    for family in std::fs::read_dir(repo.conformance("scenarios")).map_err(io)? {
        let family = family.map_err(io)?.path();
        if !family.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&family).map_err(io)? {
            let path = entry.map_err(io)?.path();
            let Some(id) = scenario_id(repo, &path) else {
                continue;
            };
            match load(repo, &id) {
                Ok(file) if file.suite.is_none() => found.push(file),
                Ok(_) | Err(RunnerError::ScenarioNotExecutable { .. }) => {}
                Err(error) => return Err(error),
            }
        }
    }
    found.sort_by(|left, right| left.scenario.cmp(&right.scenario));
    Ok(found)
}

/// The identity a `<family>/<name>.json` path declares, if it is a JSON scenario path.
fn scenario_id(repo: &Repo, path: &Path) -> Option<ScenarioId> {
    let relative = path.strip_prefix(repo.conformance("scenarios")).ok()?;
    let text = relative.to_str()?.strip_suffix(".json")?.replace('\\', "/");
    ScenarioId::parse(&text).ok()
}

/// Loads one registered scenario file and checks it against its own path.
pub(crate) fn load(repo: &Repo, scenario: &ScenarioId) -> Result<ScenarioFile, RunnerError> {
    let relative = format!("scenarios/{}", scenario.relative_path().display());
    let path = repo.conformance(&relative);
    if !path.is_file() {
        return Err(RunnerError::ScenarioMissing {
            scenario: scenario.to_string(),
        });
    }
    let value = repo.json(&relative)?;
    if value.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(RunnerError::ScenarioNotExecutable {
            scenario: scenario.to_string(),
        });
    }
    let file: ScenarioFile =
        serde_path_to_error::deserialize(value).map_err(|error| RunnerError::ScenarioInvalid {
            path: path.clone(),
            field: error.path().to_string(),
            source: error.into_inner(),
        })?;
    consistent(&file, scenario)?;
    Ok(file)
}

fn consistent(file: &ScenarioFile, scenario: &ScenarioId) -> Result<(), RunnerError> {
    let inconsistent = |detail| RunnerError::ScenarioInconsistent {
        scenario: scenario.to_string(),
        detail,
    };
    if file.format != FORMAT {
        return Err(inconsistent("the format marker is not ovis-conformance/1"));
    }
    if &file.scenario != scenario {
        return Err(inconsistent(
            "the declared scenario differs from its file path",
        ));
    }
    if file.cases.is_empty() == file.suite.is_none() {
        return Err(inconsistent("a scenario declares either cases or a suite"));
    }
    let mut names: Vec<CaseName> = file.cases.iter().map(|case| case.name).collect();
    names.sort();
    names.dedup();
    if names.len() != file.cases.len() {
        return Err(inconsistent("a case name is declared twice"));
    }
    if file.platforms.is_empty() {
        return Err(inconsistent("a scenario declares at least one platform"));
    }
    Ok(())
}

fn expand(repo: &Repo, steps: &[Step], depth: usize) -> Result<Vec<Step>, RunnerError> {
    let mut expanded = Vec::with_capacity(steps.len());
    for step in steps {
        let Step::Include(include) = step else {
            expanded.push(step.clone());
            continue;
        };
        if depth >= MAX_INCLUDE_DEPTH {
            return Err(RunnerError::ScenarioInconsistent {
                scenario: include.scenario.to_string(),
                detail: "includes nest deeper than four levels",
            });
        }
        let included = load(repo, &include.scenario)?;
        let case = included
            .cases
            .iter()
            .find(|case| case.name == include.case)
            .ok_or_else(|| RunnerError::CaseMissing {
                scenario: include.scenario.to_string(),
                case: include.case.as_str(),
            })?;
        expanded.extend(expand(repo, &case.steps, depth.saturating_add(1))?);
    }
    Ok(expanded)
}

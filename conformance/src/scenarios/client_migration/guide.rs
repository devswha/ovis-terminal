//! The migration guide's machine-read parts: one `ovis-migration` block per Ovis path and
//! the `ovis-example` replay cases it cites. Prose around them is never parsed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::RunnerError;

const ROW_FENCE: &str = "```json ovis-migration";
const EXAMPLE_FENCE: &str = "```json ovis-example";
const CLOSE_FENCE: &str = "```";

/// What a later Ovis PR is recommended to do with a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Fate {
    Replace,
    Delete,
    Adapt,
    Move,
    Keep,
}

/// One Ovis path mapped to the runtime contract that replaces or supports it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Row {
    pub(super) path: String,
    pub(super) fate: Fate,
    /// Published methods the binary serves for this path.
    pub(super) methods: Vec<String>,
    /// Published methods the binary does not serve yet: an honest gap.
    #[serde(default)]
    pub(super) unserved: Vec<String>,
    pub(super) events: Vec<String>,
    /// `capabilities.<key>` or `limits.<key>` of `hello`.
    pub(super) capabilities: Vec<String>,
    /// Schema component names of the identities the path binds to.
    pub(super) identity: Vec<String>,
    /// `fixture:<case>` of `conformance/fixtures/ovis` or `scenario:<family/name>`.
    pub(super) fixtures: Vec<String>,
    /// Ids of `ovis-example` blocks of this guide.
    pub(super) examples: Vec<String>,
}

/// Every machine-read block of the guide, in document order.
#[derive(Debug, Clone, Default)]
pub(super) struct Guide {
    pub(super) rows: Vec<Row>,
    /// Example id to its `ovis-replay/1` case document.
    pub(super) examples: Vec<(String, Value)>,
}

fn invalid(path: &Path, line: usize, source: serde_json::Error) -> RunnerError {
    RunnerError::ScenarioInvalid {
        path: path.to_path_buf(),
        field: format!("line {line}"),
        source,
    }
}

impl Guide {
    pub(super) fn read(path: &Path) -> Result<Self, RunnerError> {
        let text = std::fs::read_to_string(path).map_err(|source| RunnerError::Io {
            operation: "read the migration guide",
            source,
        })?;
        Self::parse((&text, path))
    }

    /// Parses every fenced block; a malformed block is an error at its opening line.
    pub(super) fn parse((text, path): (&str, &Path)) -> Result<Self, RunnerError> {
        let mut guide = Self::default();
        let mut open: Option<(&str, usize, String)> = None;
        for (index, line) in text.lines().enumerate() {
            let number = index.saturating_add(1);
            let trimmed = line.trim();
            match open.take() {
                None if trimmed == ROW_FENCE || trimmed == EXAMPLE_FENCE => {
                    open = Some((trimmed, number, String::new()));
                }
                None => {}
                Some((fence, start, body)) if trimmed == CLOSE_FENCE => {
                    guide.add((fence, &body), (path, start))?;
                }
                Some((fence, start, mut body)) => {
                    body.push_str(line);
                    body.push('\n');
                    open = Some((fence, start, body));
                }
            }
        }
        if let Some((_, start, _)) = open {
            return Err(RunnerError::ScenarioInconsistent {
                scenario: format!("{}:{start}", path.display()),
                detail: "a guide block is never closed",
            });
        }
        Ok(guide)
    }

    fn add(
        &mut self,
        (fence, body): (&str, &str),
        (path, line): (&Path, usize),
    ) -> Result<(), RunnerError> {
        if fence == ROW_FENCE {
            let row = serde_json::from_str(body).map_err(|source| invalid(path, line, source))?;
            self.rows.push(row);
            return Ok(());
        }
        let case: Value =
            serde_json::from_str(body).map_err(|source| invalid(path, line, source))?;
        let id = case.get("id").and_then(Value::as_str).unwrap_or_default();
        if id.is_empty() || self.examples.iter().any(|(known, _)| known == id) {
            return Err(RunnerError::ScenarioInconsistent {
                scenario: format!("{}:{line}", path.display()),
                detail: "every guide example has its own non-empty id",
            });
        }
        self.examples.push((id.to_owned(), case));
        Ok(())
    }

    /// The guide named by `--guide`, or `docs/ovis-adapter.md` of the checkout.
    pub(super) fn path(chosen: Option<&PathBuf>, repo: &Path) -> PathBuf {
        chosen.map_or_else(|| repo.join("docs").join("ovis-adapter.md"), Clone::clone)
    }
}

/// Section-6 coverage: every expected path has exactly one row with its fate, and every
/// example is defined once and cited by a row.
pub(super) fn coverage(expected: &BTreeMap<String, Fate>, guide: &Guide) -> Value {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for row in &guide.rows {
        let count = seen.entry(row.path.as_str()).or_default();
        *count = count.saturating_add(1);
    }
    let missing: Vec<&String> = expected
        .keys()
        .filter(|path| !seen.contains_key(path.as_str()))
        .collect();
    let extra: Vec<&str> = seen
        .keys()
        .copied()
        .filter(|path| !expected.contains_key(*path))
        .collect();
    let duplicate: Vec<&str> = seen
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(path, _)| *path)
        .collect();
    let wrong_fate: Vec<Value> = guide
        .rows
        .iter()
        .filter_map(|row| {
            let wanted = expected.get(&row.path)?;
            (*wanted != row.fate)
                .then(|| json!({"path": row.path, "expected": wanted, "guide": row.fate}))
        })
        .collect();
    let defined: BTreeSet<&str> = guide.examples.iter().map(|(id, _)| id.as_str()).collect();
    let cited: BTreeSet<&str> = guide
        .rows
        .iter()
        .flat_map(|row| row.examples.iter().map(String::as_str))
        .collect();
    json!({
        "missing_paths": missing,
        "extra_paths": extra,
        "duplicate_paths": duplicate,
        "wrong_fate": wrong_fate,
        "undefined_examples": cited.difference(&defined).collect::<Vec<_>>(),
        "uncited_examples": defined.difference(&cited).collect::<Vec<_>>(),
    })
}

/// The report [`coverage`] gives a complete guide.
pub(super) fn complete() -> Value {
    json!({
        "missing_paths": [],
        "extra_paths": [],
        "duplicate_paths": [],
        "wrong_fate": [],
        "undefined_examples": [],
        "uncited_examples": [],
    })
}

//! Auditing the plan: coverage of every ledger row, resolvable references, QA scenarios
//! per todo and disjoint same-wave write scopes.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::ledgers::{Ledgers, required};
use super::plan::{Plan, Todo};
use super::report::{FailureKind, Finding, Verdict};

pub(super) const PLAN_INVALID: FailureKind = FailureKind {
    kind: "plan_invalid",
    hint: "assign every uncovered row, fix each unresolved reference and split overlapping write scopes",
};

/// What the plan must cover.
#[derive(Debug, Clone)]
pub(super) struct Requirements {
    pub(super) todos: (u16, u16),
    pub(super) ideals: u16,
    pub(super) gaps: u16,
    pub(super) exclusions: u16,
    pub(super) reject_unresolved: bool,
    /// Absolute, so that a relative write scope can be compared with a denied root.
    pub(super) scope_root: PathBuf,
    pub(super) deny_write_roots: Vec<PathBuf>,
}

fn uncovered<'plan>(
    wanted: Vec<String>,
    covered: impl Iterator<Item = &'plan String>,
) -> Vec<String> {
    let covered: BTreeSet<&String> = covered.collect();
    wanted
        .into_iter()
        .filter(|id| !covered.contains(id))
        .collect()
}

fn unresolved(plan: &Plan, ledgers: &Ledgers) -> Vec<String> {
    let known: BTreeSet<String> = ledgers
        .ideal_ids()
        .into_iter()
        .chain(ledgers.gap_ids())
        .chain(ledgers.exclusion_ids())
        .collect();
    let mut found = Vec::new();
    for todo in &plan.todos {
        let edges = todo.blocked_by.iter().chain(&todo.blocks);
        for number in edges.filter(|number| !plan.accounts_for(**number)) {
            found.push(format!(
                "todo {}: todo {number} does not exist",
                todo.number
            ));
        }
        let ids = todo
            .closes
            .iter()
            .chain(&todo.delivers)
            .chain(&todo.excludes);
        for id in ids.filter(|id| !known.contains(*id)) {
            found.push(format!("todo {}: {id} is not a ledger row", todo.number));
        }
        for scenario in &todo.scenarios {
            let part_a = scenario
                .strip_prefix("part-a/task-")
                .and_then(|n| n.parse::<u16>().ok());
            if part_a.is_some_and(|number| !plan.accounts_for(number)) {
                found.push(format!("todo {}: {scenario} names no todo", todo.number));
            }
        }
    }
    found
}

fn normalized(path: &str) -> String {
    path.trim_start_matches("./")
        .trim_end_matches('/')
        .to_owned()
}

fn overlaps(left: &str, right: &str) -> bool {
    let (left, right) = (normalized(left), normalized(right));
    left == right
        || left.starts_with(&format!("{right}/"))
        || right.starts_with(&format!("{left}/"))
}

fn write_conflicts(plan: &Plan) -> Vec<String> {
    let mut found = Vec::new();
    for (index, first) in plan.todos.iter().enumerate() {
        for second in plan.todos.iter().skip(index.saturating_add(1)) {
            let Some(wave) = first.wave.filter(|wave| second.wave == Some(*wave)) else {
                continue;
            };
            for path in &first.write_scope {
                if let Some(other) = second
                    .write_scope
                    .iter()
                    .find(|other| overlaps(path, other))
                {
                    found.push(format!(
                        "wave {wave}: todo {} {path} overlaps todo {} {other}",
                        first.number, second.number
                    ));
                }
            }
        }
    }
    found
}

fn denied_writes(todo: &Todo, requirements: &Requirements) -> Vec<String> {
    todo.write_scope
        .iter()
        .filter(|path| {
            let target = requirements.scope_root.join(Path::new(path.as_str()));
            requirements
                .deny_write_roots
                .iter()
                .any(|deny| target.starts_with(deny))
        })
        .map(|path| format!("todo {}: {path} is under a denied write root", todo.number))
        .collect()
}

fn missing_scenarios(plan: &Plan) -> Vec<String> {
    plan.todos
        .iter()
        .filter(|todo| {
            let owns_rows = !todo.closes.is_empty() || !todo.delivers.is_empty();
            !todo.has_qa || (owns_rows && todo.scenarios.is_empty())
        })
        .map(|todo| format!("todo {} (line {})", todo.number, todo.line))
        .collect()
}

fn flag(verdict: &mut Verdict, (rule, key): (&'static str, &str), items: &[String]) {
    for item in items {
        verdict
            .findings
            .push(Finding::new(rule, item.clone(), key.replace('_', " ")));
    }
    verdict.fact(key, json!(items));
}

/// The plan verdict with the F1 report keys as facts.
pub(super) fn audit(plan: &Plan, ledgers: &Ledgers, requirements: &Requirements) -> Verdict {
    let mut verdict = Verdict::new("plan-audit", PLAN_INVALID);
    let todos = &plan.todos;
    let ideals = uncovered(
        required("IDEAL", requirements.ideals),
        todos.iter().flat_map(|todo| &todo.delivers),
    );
    let gaps = uncovered(
        required("GAP", requirements.gaps),
        todos.iter().flat_map(|todo| &todo.closes),
    );
    let excluded = uncovered(
        required("EXCL", requirements.exclusions),
        todos.iter().flat_map(|todo| &todo.excludes),
    );
    let (first, last) = requirements.todos;
    let missing_todos: Vec<String> = (first..=last)
        .filter(|number| !plan.accounts_for(*number))
        .map(|number| format!("todo {number}"))
        .collect();
    flag(
        &mut verdict,
        ("uncovered-ideal", "uncovered_ideals"),
        &ideals,
    );
    flag(&mut verdict, ("uncovered-gap", "uncovered_gaps"), &gaps);
    flag(
        &mut verdict,
        ("excluded-feature", "excluded_features"),
        &excluded,
    );
    flag(
        &mut verdict,
        ("missing-todo", "missing_todos"),
        &missing_todos,
    );
    flag(
        &mut verdict,
        ("missing-scenario", "missing_scenarios"),
        &missing_scenarios(plan),
    );
    flag(
        &mut verdict,
        ("write-conflict", "write_conflicts"),
        &write_conflicts(plan),
    );
    let denied: Vec<String> = todos
        .iter()
        .flat_map(|todo| denied_writes(todo, requirements))
        .collect();
    flag(&mut verdict, ("denied-write", "denied_writes"), &denied);
    let references = unresolved(plan, ledgers);
    if requirements.reject_unresolved {
        flag(
            &mut verdict,
            ("unresolved-ref", "unresolved_refs"),
            &references,
        );
    } else {
        verdict.fact("unresolved_refs", json!(references));
    }
    let landed: Vec<Value> = todos
        .iter()
        .filter(|todo| todo.landed)
        .map(|todo| json!(todo.number.to_string()))
        .collect();
    verdict.fact("todos", json!(todos.len().to_string()));
    verdict.fact("landed", Value::Array(landed));
    verdict.fact(
        "removed",
        json!(
            plan.removed
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        ),
    );
    verdict
}

//! The review checklist: one disposition per Rule totals id, validated against the
//! repository and rendered to `docs/quality-checklist.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

use super::cargo_rules::{CARGO_RULES, lint_table};
use super::error::QualityError;
use super::report::Finding;
use super::rules::SOURCE_RULES;
use super::violations::Violations;

/// The checked-in dispositions every rendered checklist comes from.
pub(super) const DISPOSITIONS: &str = include_str!("dispositions.json");
const FORMAT: &str = "ovis-quality-dispositions/1";
const AUDIT_GATES: &[&str] = &["evidence-audit", "plan-audit", "ideal-audit"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Dispositions {
    format: String,
    pub(super) precedence: String,
    pub(super) rules: Vec<Disposition>,
    pub(super) exclusions: Vec<ExclusionCheck>,
    pub(super) dropped_result_exemptions: Vec<super::discards::DroppedResultExemption>,
}

/// An EXCL row of `ideal-state.md` this todo must not reproduce, and what enforces it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExclusionCheck {
    pub(super) id: String,
    pub(super) enforced_by: Vec<String>,
    pub(super) check: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Status {
    Applicable,
    NotApplicable,
}

impl Status {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Applicable => "applicable",
            Self::NotApplicable => "not applicable",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Disposition {
    pub(super) rule: String,
    pub(super) status: Status,
    pub(super) enforced_by: Vec<String>,
    pub(super) check: String,
    #[serde(default)]
    pub(super) cited: Vec<String>,
}

/// Parses the embedded dispositions; a malformed table is itself a finding.
pub(super) fn dispositions() -> Result<Dispositions, Finding> {
    let parsed: Dispositions = serde_json::from_str(DISPOSITIONS).map_err(|error| {
        Finding::new(
            "checklist-disposition",
            "probes/quality/dispositions.json",
            error.to_string(),
        )
    })?;
    if parsed.format == FORMAT {
        Ok(parsed)
    } else {
        let detail = format!("format is {:?}, not {FORMAT:?}", parsed.format);
        Err(Finding::new(
            "checklist-disposition",
            "probes/quality/dispositions.json",
            detail,
        ))
    }
}

/// What gate references are checked against.
pub(super) struct RepoFacts<'repo> {
    repo: &'repo Path,
    clippy: BTreeMap<String, String>,
    rust: BTreeMap<String, String>,
    clippy_toml: String,
    ideal_state: String,
}

impl<'repo> RepoFacts<'repo> {
    pub(super) fn read(repo: &'repo Path) -> Result<Self, QualityError> {
        let text = |name: &str| {
            let path = repo.join(name);
            std::fs::read_to_string(&path).map_err(QualityError::io(&path))
        };
        let manifest = text("Cargo.toml")?;
        Ok(Self {
            repo,
            clippy: lint_table(&manifest, "workspace.lints.clippy"),
            rust: lint_table(&manifest, "workspace.lints.rust"),
            clippy_toml: text("clippy.toml")?,
            ideal_state: text("docs/design/ideal-state.md")?,
        })
    }

    /// Whether `token` names a gate that exists in this repository.
    fn resolves(&self, token: &str) -> bool {
        let enabled = |table: &BTreeMap<String, String>, name: &str| {
            table
                .get(name)
                .is_some_and(|level| level == "deny" || level == "warn")
        };
        let Some((kind, name)) = token.split_once("::") else {
            return token == "review";
        };
        match kind {
            "clippy" | "clippy-group" => enabled(&self.clippy, name),
            "rust" => enabled(&self.rust, name),
            "clippy.toml" => self.clippy_toml.lines().any(|line| {
                line.split_once('=')
                    .is_some_and(|(key, _)| key.trim() == name)
            }),
            "source-check" => SOURCE_RULES.contains(&name) || CARGO_RULES.contains(&name),
            "gate" => AUDIT_GATES.contains(&name),
            "scenario" => self
                .repo
                .join("conformance/scenarios")
                .join(format!("{name}.json"))
                .is_file(),
            "file" => self.repo.join(name).exists(),
            _ => false,
        }
    }
}

fn check_one(rule: &Disposition, violations: &Violations, facts: &RepoFacts<'_>) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut flag = |detail: String| {
        found.push(Finding::new(
            "checklist-disposition",
            rule.rule.clone(),
            detail,
        ));
    };
    let Some(total) = violations.rule(&rule.rule) else {
        return found;
    };
    match (rule.status, rule.enforced_by.is_empty()) {
        (Status::Applicable, true) => flag("an applicable rule names no gate or review".to_owned()),
        (Status::NotApplicable, false) => flag("a not-applicable rule names gates".to_owned()),
        (Status::Applicable | Status::NotApplicable, _) => {}
    }
    if rule.check.trim().is_empty() {
        flag("the check or justification is empty".to_owned());
    }
    for token in rule
        .enforced_by
        .iter()
        .filter(|token| !facts.resolves(token))
    {
        flag(format!("gate {token} does not exist in this repository"));
    }
    for cited in rule.cited.iter().filter(|id| !total.v_rows.contains(id)) {
        flag(format!("cited {cited} is not a V-row of this rule"));
    }
    found
}

/// Every ledger rule has exactly one disposition and every disposition a ledger rule.
pub(super) fn coverage(table: &Dispositions, violations: &Violations) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    for rule in &table.rules {
        if !seen.insert(rule.rule.as_str()) {
            found.push(Finding::new(
                "checklist-disposition",
                rule.rule.clone(),
                "listed twice",
            ));
        }
        if violations.rule(&rule.rule).is_none() {
            found.push(Finding::new(
                "checklist-disposition",
                rule.rule.clone(),
                "no Rule totals row has this id",
            ));
        }
    }
    for total in violations
        .rules
        .iter()
        .filter(|total| !seen.contains(total.id.as_str()))
    {
        found.push(Finding::new(
            "checklist-disposition",
            total.id.clone(),
            "Rule totals id without a disposition",
        ));
    }
    found
}

/// Coverage plus every disposition's status, gates and cited V-rows.
pub(super) fn validate(
    table: &Dispositions,
    violations: &Violations,
    facts: &RepoFacts<'_>,
) -> Vec<Finding> {
    let mut found = coverage(table, violations);
    for rule in &table.rules {
        found.extend(check_one(rule, violations, facts));
    }
    for exclusion in &table.exclusions {
        let mut flag = |detail: String| {
            found.push(Finding::new(
                "checklist-disposition",
                exclusion.id.clone(),
                detail,
            ));
        };
        if !facts.ideal_state.contains(&format!("| {} |", exclusion.id)) {
            flag("not an EXCL row of docs/design/ideal-state.md".to_owned());
        }
        if exclusion.enforced_by.is_empty() || exclusion.check.trim().is_empty() {
            flag("an exclusion needs a gate and a check".to_owned());
        }
        for token in exclusion
            .enforced_by
            .iter()
            .filter(|token| !facts.resolves(token))
        {
            flag(format!("gate {token} does not exist in this repository"));
        }
    }
    found
}

//! `docs/research/violations.md`: the Rule totals table and every V-row it counts.

use std::collections::BTreeMap;

use super::markdown::{cells, section};
use super::report::Finding;

/// One Rule totals row and the V-rows that carry its rule id.
#[derive(Debug, Clone)]
pub(super) struct RuleTotal {
    pub(super) id: String,
    pub(super) rows: usize,
    pub(super) severity: String,
    pub(super) instead: String,
    pub(super) v_rows: Vec<String>,
}

/// The parsed ledger, in Rule totals order.
#[derive(Debug, Clone)]
pub(super) struct Violations {
    pub(super) rules: Vec<RuleTotal>,
    pub(super) total_rows: usize,
}

impl Violations {
    pub(super) fn rule(&self, id: &str) -> Option<&RuleTotal> {
        self.rules.iter().find(|rule| rule.id == id)
    }
}

const TOTALS: &str = "## Rule totals";

/// `V-123` and `V-RT-HC-01` name V-rows; the header cell `V-row` does not occur.
fn is_v_row(id: &str) -> bool {
    id.strip_prefix("V-").is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '-')
    })
}

/// Parses the ledger and reports every inconsistency between Rule totals and V-rows.
pub(super) fn parse(text: &str) -> (Violations, Vec<Finding>) {
    let mut findings = Vec::new();
    let mut by_rule: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut total_rows = 0_usize;
    for row in text.lines().filter_map(cells) {
        if let [id, rule, ..] = row.as_slice()
            && is_v_row(id)
        {
            by_rule
                .entry((*rule).to_owned())
                .or_default()
                .push((*id).to_owned());
            total_rows = total_rows.saturating_add(1);
        }
    }
    let mut rules = Vec::new();
    let table = section(text, TOTALS).unwrap_or_default();
    for row in table.iter().filter_map(|line| cells(line)) {
        let [id, rows, _, severity, instead] = row.as_slice() else {
            continue;
        };
        let Ok(rows) = rows.parse::<usize>() else {
            continue;
        };
        let v_rows = by_rule.remove(*id).unwrap_or_default();
        if v_rows.len() != rows {
            let detail = format!(
                "Rule totals counts {rows} rows, the ledger has {}",
                v_rows.len()
            );
            findings.push(Finding::new("checklist-ledger", (*id).to_owned(), detail));
        }
        rules.push(RuleTotal {
            id: (*id).to_owned(),
            rows,
            severity: (*severity).to_owned(),
            instead: (*instead).to_owned(),
            v_rows,
        });
    }
    if rules.is_empty() {
        findings.push(Finding::new(
            "checklist-ledger",
            TOTALS,
            "no Rule totals rows",
        ));
    }
    for (rule, orphans) in by_rule {
        let detail = format!(
            "{} V-rows carry a rule id missing from Rule totals",
            orphans.len()
        );
        findings.push(Finding::new("checklist-ledger", rule, detail));
    }
    (Violations { rules, total_rows }, findings)
}

//! Rendering `docs/quality-checklist.md` from the ledger and the dispositions.

use super::checklist::{Disposition, Dispositions, Status};
use super::violations::Violations;

/// The command that regenerates the checklist; `source-check` fails on any difference.
pub(super) const REGENERATE: &str = "cargo run --locked --manifest-path probes/Cargo.toml --bin source-check -- --roots crates packages/client-ts conformance bench probes --violations docs/research/violations.md --checklist docs/quality-checklist.md --write-checklist";

fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

fn gates(rule: &Disposition) -> String {
    if rule.enforced_by.is_empty() {
        return "-".to_owned();
    }
    rule.enforced_by
        .iter()
        .map(|token| format!("`{token}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn mechanical(rule: &Disposition) -> bool {
    rule.enforced_by.iter().any(|token| token != "review")
}

fn summary(table: &Dispositions, violations: &Violations) -> String {
    let applicable = table
        .rules
        .iter()
        .filter(|rule| rule.status == Status::Applicable);
    let gated = applicable.clone().filter(|rule| mechanical(rule)).count();
    let review = applicable.count().saturating_sub(gated);
    let excused = table
        .rules
        .iter()
        .filter(|rule| rule.status == Status::NotApplicable)
        .count();
    format!(
        "{} rule ids, {} V-rows: {gated} applicable with a mechanical gate, {review} applicable as review items, {excused} not applicable with a justification.",
        violations.rules.len(),
        violations.total_rows,
    )
}

/// The complete checklist text, deterministic for one ledger and one dispositions table.
pub(super) fn render(table: &Dispositions, violations: &Violations) -> String {
    let mut out = String::new();
    let mut line = |text: &str| {
        out.push_str(text);
        out.push('\n');
    };
    line("# Quality checklist");
    line("");
    line(
        "Generated from the Rule totals of `docs/research/violations.md` and `probes/quality/dispositions.json`; do not edit by hand. Regenerate with:",
    );
    line("");
    line(&format!("    {REGENERATE}"));
    line("");
    line(
        "`source-check` exits 1 when this file differs from what that command writes, when a Rule totals id has no disposition, or when a named gate does not exist.",
    );
    line("");
    line(&summary(table, violations));
    line("");
    line(&format!("Precedence: {}", table.precedence));
    line("");
    line(
        "Gate tokens: `clippy::<lint>` and `rust::<lint>` are workspace lint-table entries (warnings fail `-D warnings`), `clippy-group::<group>` a lint group, `clippy.toml::<key>` a clippy threshold, `source-check::<rule>` a rule of this checker, `gate::<tool>` a quality auditor, `scenario::<id>` a conformance scenario, `file::<path>` a checked-in gate file, `review` a reviewer question.",
    );
    line("");
    line("## Checklist");
    line("");
    line(
        "| rule | V-rows | severity | status | enforced by | contract (violations.md) | check or justification |",
    );
    line("|---|---:|---|---|---|---|---|");
    for total in &violations.rules {
        let Some(rule) = table.rules.iter().find(|rule| rule.rule == total.id) else {
            continue;
        };
        line(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |",
            total.id,
            total.rows,
            total.severity,
            rule.status.label(),
            gates(rule),
            cell(&total.instead),
            cell(&rule.check)
        ));
    }
    line("");
    line("## V-rows cited by todo 61");
    line("");
    line(
        "These legacy defects must never be reproduced; each is enforced by the gate of its rule.",
    );
    line("");
    line("| V-row | rule | enforced by |");
    line("|---|---|---|");
    for rule in table.rules.iter().filter(|rule| !rule.cited.is_empty()) {
        for cited in &rule.cited {
            line(&format!("| {cited} | {} | {} |", rule.rule, gates(rule)));
        }
    }
    line("");
    line("## Exclusions enforced by todo 61");
    line("");
    line("| EXCL row | enforced by | check |");
    line("|---|---|---|");
    for exclusion in &table.exclusions {
        let tokens: Vec<String> = exclusion
            .enforced_by
            .iter()
            .map(|token| format!("`{token}`"))
            .collect();
        line(&format!(
            "| {} | {} | {} |",
            exclusion.id,
            tokens.join(", "),
            cell(&exclusion.check)
        ));
    }
    line("");
    line("## V-row references");
    for total in &violations.rules {
        let joined = total.v_rows.join(", ");
        line(&format!(
            "\n### {} ({})\n\n{joined}",
            total.id,
            total.v_rows.len()
        ));
    }
    out
}

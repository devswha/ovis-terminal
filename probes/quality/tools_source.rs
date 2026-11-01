//! `source-check`: every source rule over the roots, the Cargo configuration and the
//! checklist freshness against `docs/research/violations.md`.

use clap::Parser as _;
use serde_json::json;

use super::args::SourceCheckArgs;
use super::cargo_rules::{CARGO_RULES, check_members, check_workspace};
use super::checklist::{RepoFacts, dispositions, validate};
use super::cli::Outcome;
use super::discards::apply_exemptions;
use super::error::QualityError;
use super::matrix_build::sha256_hex;
use super::render::{REGENERATE, render};
use super::report::{FailureKind, Finding, Verdict, count};
use super::rules::SOURCE_RULES;
use super::scan::{SourceLimits, scan_roots};
use super::{seeds, violations};

pub(super) const QUALITY_VIOLATION: FailureKind = FailureKind {
    kind: "quality_violation",
    hint: "fix each finding structurally; limits are never raised and lints never silenced",
};

fn checklist(args: &SourceCheckArgs, verdict: &mut Verdict) -> Result<(), QualityError> {
    let ledger_path = args.repo.join(&args.violations);
    let ledger = std::fs::read_to_string(&ledger_path).map_err(QualityError::io(&ledger_path))?;
    let (parsed, problems) = violations::parse(&ledger);
    verdict.findings.extend(problems);
    let table = match dispositions() {
        Ok(table) => table,
        Err(finding) => {
            verdict.findings.push(finding);
            return Ok(());
        }
    };
    let facts = RepoFacts::read(&args.repo)?;
    verdict.findings.extend(validate(&table, &parsed, &facts));
    let rendered = render(&table, &parsed);
    let path = args.repo.join(&args.checklist);
    if args.write_checklist {
        std::fs::write(&path, &rendered).map_err(QualityError::io(&path))?;
    }
    let current = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(QualityError::io(&path)(error)),
    };
    if current.as_deref() != Some(rendered.as_str()) {
        let detail = format!("differs from the generated checklist; regenerate with: {REGENERATE}");
        verdict.findings.push(Finding::new(
            "checklist-stale",
            args.checklist.to_string_lossy(),
            detail,
        ));
    }
    verdict.fact(
        "checklist",
        json!({
            "path": args.checklist.to_string_lossy(),
            "sha256": sha256_hex(rendered.as_bytes()),
            "rule_ids": count(parsed.rules.len()),
            "v_rows": count(parsed.total_rows),
            "violations_sha256": sha256_hex(ledger.as_bytes()),
        }),
    );
    Ok(())
}

pub(super) fn run(command_line: &[String]) -> Result<Outcome, QualityError> {
    let args = SourceCheckArgs::try_parse_from(command_line)
        .map_err(|error| QualityError::from_clap(&error))?;
    let limits = SourceLimits::tightened(args.max_pure_loc, args.max_params)?;
    let mut verdict = Verdict::new("source-check", QUALITY_VIOLATION);
    verdict.fact(
        "limits",
        json!({"max_pure_loc": count(limits.pure_loc), "max_params": count(limits.params)}),
    );
    verdict.fact(
        "rules",
        json!({"source": SOURCE_RULES, "cargo": CARGO_RULES}),
    );
    if let Some(name) = &args.fixture {
        let (path, findings) = seeds::source(name, limits)?;
        verdict.findings.extend(findings);
        verdict.fact("fixture", json!({"name": name, "path": path}));
        return Ok(Outcome {
            verdict,
            output: args.evidence,
        });
    }
    let scanned = scan_roots(&args.repo, &args.roots, limits)?;
    match dispositions() {
        Ok(table) => verdict
            .findings
            .extend(apply_exemptions(&table, scanned.findings.clone())),
        Err(finding) => {
            verdict.findings.extend(scanned.findings.iter().cloned());
            verdict.findings.push(finding);
        }
    }
    verdict.fact("sources", scanned.facts());
    verdict.findings.extend(check_workspace(&args.repo)?);
    verdict
        .findings
        .extend(check_members(&args.repo, &args.roots)?);
    checklist(&args, &mut verdict)?;
    Ok(Outcome {
        verdict,
        output: args.evidence,
    })
}

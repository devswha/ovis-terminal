//! `evidence-audit`, `plan-audit` and `ideal-audit`: argument handling and inputs.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::args::{EvidenceAuditArgs, IdealAuditArgs, PlanAuditArgs};
use super::cli::Outcome;
use super::error::QualityError;
use super::evidence::{EVIDENCE_INVALID, audit_text, runs};
use super::ideal_audit::{self, IdealRequirements};
use super::ledgers::Ledgers;
use super::matrix::IdealMap;
use super::plan::parse;
use super::plan_audit::{self, Requirements};
use super::report::{Finding, Verdict, count};
use super::seeds;

fn parsed<T: clap::Parser>(command_line: &[String]) -> Result<T, QualityError> {
    T::try_parse_from(command_line).map_err(|error| QualityError::from_clap(&error))
}

fn read(path: &Path) -> Result<String, QualityError> {
    std::fs::read_to_string(path).map_err(QualityError::io(path))
}

fn absolute(path: &Path) -> Result<PathBuf, QualityError> {
    std::path::absolute(path).map_err(QualityError::io(path))
}

pub(super) fn evidence(command_line: &[String]) -> Result<Outcome, QualityError> {
    let args: EvidenceAuditArgs = parsed(command_line)?;
    let mut verdict = Verdict::new("evidence-audit", EVIDENCE_INVALID);
    let documents: Vec<(String, String)> = match &args.fixture {
        Some(name) => vec![seeds::evidence(name)?],
        None => args
            .inputs
            .iter()
            .map(|path| Ok((path.to_string_lossy().into_owned(), read(path)?)))
            .collect::<Result<_, QualityError>>()?,
    };
    let mut audited = 0_usize;
    for (label, text) in &documents {
        audited = audited.saturating_add(runs(text).map_or(0, |runs| runs.len()));
        verdict.findings.extend(audit_text(label, text));
    }
    let labels: Vec<&String> = documents.iter().map(|(label, _)| label).collect();
    verdict.fact("inputs", json!(labels));
    verdict.fact("runs", count(audited));
    Ok(Outcome {
        verdict,
        output: args.evidence,
    })
}

fn todo_range(text: &str) -> Result<(u16, u16), QualityError> {
    let usage = || QualityError::Usage {
        detail: format!("--require-todos takes FIRST-LAST, not {text}"),
    };
    let (first, last) = text.split_once('-').ok_or_else(usage)?;
    Ok((
        first.parse().map_err(|_| usage())?,
        last.parse().map_err(|_| usage())?,
    ))
}

pub(super) fn plan(command_line: &[String]) -> Result<Outcome, QualityError> {
    let args: PlanAuditArgs = parsed(command_line)?;
    let (plan_text, ledgers, requirements) = if let Some(name) = &args.fixture {
        seeds::plan(name)?
    } else {
        let plan_path = args.plan.clone().ok_or_else(|| QualityError::Usage {
            detail: "--plan is required".to_owned(),
        })?;
        let requirements = Requirements {
            todos: todo_range(&args.require_todos)?,
            ideals: args.require_ideals,
            gaps: args.require_gaps,
            exclusions: args.require_exclusions,
            reject_unresolved: args.reject_unresolved,
            scope_root: absolute(&args.scope_root)?,
            deny_write_roots: args
                .deny_write_root
                .iter()
                .map(|root| absolute(root))
                .collect::<Result<_, _>>()?,
        };
        (
            read(&plan_path)?,
            Ledgers::read(&args.ideals, &args.gaps)?,
            requirements,
        )
    };
    let verdict = plan_audit::audit(&parse(&plan_text), &ledgers, &requirements);
    Ok(Outcome {
        verdict,
        output: args.evidence,
    })
}

fn evidence_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), QualityError> {
    for entry in std::fs::read_dir(dir).map_err(QualityError::io(dir))? {
        let path = entry.map_err(QualityError::io(dir))?.path();
        if path.is_dir() {
            evidence_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            out.push(path);
        }
    }
    Ok(())
}

/// Labelled runs, and a finding for each JSON file that holds no run.
type Loaded = (Vec<(String, Value)>, Vec<Finding>);

/// Every run found under `dir`; a JSON file that holds no run is a finding.
fn load_runs(dir: &Path) -> Result<Loaded, QualityError> {
    let mut paths = Vec::new();
    evidence_files(dir, &mut paths)?;
    paths.sort();
    let mut loaded = Vec::new();
    let mut unreadable = Vec::new();
    for path in paths {
        let label = path.to_string_lossy().into_owned();
        match runs(&read(&path)?) {
            Some(found) => loaded.extend(
                found
                    .into_iter()
                    .enumerate()
                    .map(|(index, run)| (format!("{label}#{index}"), run)),
            ),
            None => unreadable.push(Finding::new(
                "evidence-invalid",
                label,
                "not a JSON evidence document",
            )),
        }
    }
    Ok((loaded, unreadable))
}

pub(super) fn ideal(command_line: &[String]) -> Result<Outcome, QualityError> {
    let args: IdealAuditArgs = parsed(command_line)?;
    let (map, ledgers, (runs, unreadable), requirements) = if let Some(name) = &args.fixture {
        let (map, ledgers, runs, requirements) = seeds::ideal(name)?;
        (map, ledgers, (runs, Vec::new()), requirements)
    } else {
        let mapping = read(&args.mapping)?;
        let map: IdealMap =
            serde_json::from_str(&mapping).map_err(|source| QualityError::Json {
                path: args.mapping.clone(),
                source,
            })?;
        let dir = args
            .evidence_dir
            .clone()
            .ok_or_else(|| QualityError::Usage {
                detail: "--evidence-dir is required".to_owned(),
            })?;
        let requirements = IdealRequirements {
            rows: args.require_rows,
            platforms: args
                .require_platforms
                .iter()
                .map(|platform| platform.os().to_owned())
                .collect(),
            windows_smoke: args.windows_contract_smoke,
            soak_seconds: args.require_continuous_soak_seconds,
        };
        (
            map,
            Ledgers::read(&args.ideals, &args.gaps)?,
            load_runs(&dir)?,
            requirements,
        )
    };
    let mut verdict = ideal_audit::audit(&map, &ledgers, (&runs, &requirements));
    verdict.findings.extend(unreadable);
    verdict.fact("runs", count(runs.len()));
    Ok(Outcome {
        verdict,
        output: args.report,
    })
}

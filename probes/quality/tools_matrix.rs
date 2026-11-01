//! `quality-matrix`: writing, checking or structurally verifying the two matrices.

use std::path::Path;

use clap::Parser as _;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::args::{MatrixArgs, MatrixMode};
use super::cli::Outcome;
use super::error::QualityError;
use super::ledgers::Ledgers;
use super::matrix::{IdealMap, Sources, Traceability, verify};
use super::matrix_build::{Inputs, build, sha256_hex};
use super::plan::parse;
use super::report::{FailureKind, Finding, Verdict, count};

const MATRIX_STALE: FailureKind = FailureKind {
    kind: "matrix_invalid",
    hint: "regenerate with quality-matrix --mode write --plan <plan> and review the diff",
};

fn read(path: &Path) -> Result<String, QualityError> {
    std::fs::read_to_string(path).map_err(QualityError::io(path))
}

fn load<T: DeserializeOwned>(path: &Path) -> Result<T, QualityError> {
    serde_json::from_str(&read(path)?).map_err(|source| QualityError::Json {
        path: path.to_path_buf(),
        source,
    })
}

fn store<T: Serialize>(path: &Path, value: &T) -> Result<(), QualityError> {
    let mut text = serde_json::to_string_pretty(value).map_err(|source| QualityError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    text.push('\n');
    std::fs::write(path, text).map_err(QualityError::io(path))
}

fn generated(
    args: &MatrixArgs,
    ledgers: &Ledgers,
) -> Result<(Traceability, IdealMap), QualityError> {
    let plan_path = args.plan.clone().ok_or_else(|| QualityError::Usage {
        detail: "--plan is required by write and check".to_owned(),
    })?;
    let plan_text = read(&plan_path)?;
    let schema_text = read(&args.schema)?;
    let command = format!(
        "cargo run --locked --manifest-path probes/Cargo.toml --bin quality-matrix -- --mode write --plan {}",
        args.plan_label
    );
    let sources = Sources {
        plan: args.plan_label.clone(),
        plan_sha256: sha256_hex(plan_text.as_bytes()),
        ideal_state_sha256: sha256_hex(read(&args.ideals)?.as_bytes()),
        gap_list_sha256: sha256_hex(read(&args.gaps)?.as_bytes()),
        command,
    };
    let plan = parse(&plan_text);
    let schema_path = args.schema.to_string_lossy();
    let inputs = Inputs {
        plan: &plan,
        ledgers,
        schema_path: &schema_path,
        schema_text: &schema_text,
        sources,
    };
    build(&inputs).map_err(|source| QualityError::Json {
        path: args.schema.clone(),
        source,
    })
}

fn unowned(trace: &Traceability) -> Value {
    let sections = [
        ("ideals", &trace.ideals),
        ("gaps", &trace.gaps),
        ("exclusions", &trace.exclusions),
    ];
    let listed: serde_json::Map<String, Value> = sections
        .into_iter()
        .map(|(name, section)| {
            let ids: Vec<&String> = section
                .iter()
                .filter(|(_, owners)| owners.todos.is_empty())
                .map(|(id, _)| id)
                .collect();
            (name.to_owned(), json!(ids))
        })
        .collect();
    Value::Object(listed)
}

pub(super) fn run(command_line: &[String]) -> Result<Outcome, QualityError> {
    let args = MatrixArgs::try_parse_from(command_line)
        .map_err(|error| QualityError::from_clap(&error))?;
    let ledgers = Ledgers::read(&args.ideals, &args.gaps)?;
    let mut verdict = Verdict::new("quality-matrix", MATRIX_STALE);
    if args.mode == MatrixMode::Write {
        let (trace, map) = generated(&args, &ledgers)?;
        store(&args.traceability, &trace)?;
        store(&args.ideal_map, &map)?;
    }
    let trace: Traceability = load(&args.traceability)?;
    let map: IdealMap = load(&args.ideal_map)?;
    if args.mode == MatrixMode::Check {
        let (fresh_trace, fresh_map) = generated(&args, &ledgers)?;
        if fresh_trace != trace {
            verdict.findings.push(Finding::new(
                "matrix-stale",
                args.traceability.to_string_lossy(),
                "differs from the regenerated matrix",
            ));
        }
        if fresh_map != map {
            verdict.findings.push(Finding::new(
                "matrix-stale",
                args.ideal_map.to_string_lossy(),
                "differs from the regenerated map",
            ));
        }
    }
    let schema: Value = load(&args.schema)?;
    verdict
        .findings
        .extend(verify((&trace, &map), &ledgers, &schema));
    verdict.fact("ideals", count(map.ideals.len()));
    verdict.fact("gaps", count(map.gaps.len()));
    verdict.fact("exclusions", count(map.exclusions.len()));
    verdict.fact("methods", count(map.contract.methods.len()));
    verdict.fact("unowned", unowned(&trace));
    Ok(Outcome {
        verdict,
        output: None,
    })
}

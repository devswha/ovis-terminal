//! One runner invocation: plan, run every case, write the evidence, choose the exit status.

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::json;
use tokio::time::Instant;

use crate::cli::{CaseSelector, Cli};
use crate::engine::run::run_case;
use crate::engine::session::Context;
use crate::error::RunnerError;
use crate::evidence::{
    BinaryRecord, CaseReport, Evidence, Failure, Host, Outcome, Totals, sha256_hex,
};
use crate::ids::{CaseName, Decimal, ScenarioId};
use crate::report::{summary, transcript, write_evidence};
use crate::scenario::inventory::Inventory;
use crate::scenario::load::{Repo, plan};

const USAGE_EXIT: u8 = 2;

#[derive(Debug)]
struct Invocation {
    binary: PathBuf,
    scenario: ScenarioId,
    case: Option<CaseName>,
    evidence: PathBuf,
    require_no_skips: bool,
    manual_transcript: bool,
    scenario_root: Option<PathBuf>,
    fixtures: Option<PathBuf>,
    inject: Option<serde_json::Value>,
    append: bool,
}

impl Invocation {
    fn from_cli(cli: Cli) -> Result<Self, RunnerError> {
        let missing = |detail| RunnerError::Usage { detail };
        let binary = cli.binary.ok_or_else(|| missing("--binary is required"))?;
        let scenario = cli
            .scenario
            .ok_or_else(|| missing("--scenario is required"))?;
        let evidence = cli
            .output
            .evidence
            .ok_or_else(|| missing("--evidence is required"))?;
        if !cli.isolated {
            return Err(RunnerError::IsolationRequired);
        }
        let inject = match cli.inject {
            None => None,
            Some(text) => Some(injected(text).ok_or_else(|| {
                missing("--inject takes one JSON object or one ovis-terminal command line")
            })?),
        };
        Ok(Self {
            binary,
            scenario: ScenarioId::parse(&scenario)?,
            case: match cli.case {
                CaseSelector::Happy => Some(CaseName::Happy),
                CaseSelector::Failure => Some(CaseName::Failure),
                CaseSelector::All => None,
            },
            evidence,
            require_no_skips: cli.require_no_skips,
            manual_transcript: cli.manual_transcript,
            scenario_root: cli.scenario_root,
            fixtures: cli.fixtures,
            inject,
            append: cli.output.append,
        })
    }
}

/// A JSON object injects a request; text that is not JSON and does not start like JSON
/// injects the planned command line it equals. Malformed JSON stays a usage error.
fn injected(text: String) -> Option<serde_json::Value> {
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => value.is_object().then_some(value),
        Err(_) if text.trim_start().starts_with(['{', '[']) => None,
        Err(_) => Some(serde_json::Value::String(text)),
    }
}

fn top_failure(error: &RunnerError) -> Failure {
    Failure {
        kind: error.kind(),
        hint: error.hint(),
        detail: error.to_string(),
        step: None,
    }
}

fn binary_record(binary: &std::path::Path) -> Result<BinaryRecord, RunnerError> {
    let bytes = std::fs::read(binary).map_err(|source| RunnerError::BinaryMissing {
        path: binary.to_path_buf(),
        source,
    })?;
    Ok(BinaryRecord {
        path: binary.display().to_string(),
        sha256: sha256_hex(&bytes),
        bytes: Decimal::from_usize(bytes.len()),
    })
}

/// Runs the invocation and reports it; exit 0 only when every planned case passed.
pub(crate) async fn main(cli: Cli) -> ExitCode {
    let started = Instant::now();
    let invocation = match Invocation::from_cli(cli) {
        Ok(invocation) => invocation,
        Err(error) => {
            summary(&json!({"result": "usage", "error": top_failure(&error)}));
            return ExitCode::from(USAGE_EXIT);
        }
    };
    let (binary, cases, inventory, error) = match prepare(&invocation).await {
        Ok(Prepared {
            binary,
            cases,
            inventory,
        }) => {
            let declared = inventory
                .as_ref()
                .map_or(0, |inventory| inventory.declared_only.len());
            let problem = inventory.as_ref().and_then(Inventory::problem).or_else(|| {
                (invocation.require_no_skips && declared > 0)
                    .then_some(RunnerError::DeclaredOnly { count: declared })
            });
            (
                Some(binary),
                cases,
                inventory,
                problem.as_ref().map(top_failure),
            )
        }
        Err(error) => (None, Vec::new(), None, Some(top_failure(&error))),
    };
    let mut totals = Totals::of(&cases);
    totals.declared_only = Decimal::from_usize(
        inventory
            .as_ref()
            .map_or(0, |inventory| inventory.declared_only.len()),
    );
    let failed = failed(error.as_ref(), &totals, invocation.require_no_skips);
    let evidence = Evidence {
        format: "ovis-conformance-evidence/1",
        runner_version: env!("CARGO_PKG_VERSION"),
        host: Host::current(),
        binary,
        scenario: invocation.scenario.clone(),
        case: invocation.case.map_or("all", CaseName::as_str),
        result: if failed {
            Outcome::Failed
        } else {
            Outcome::Passed
        },
        totals,
        error,
        inventory,
        elapsed_ms: Decimal(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
        cases,
    };
    if invocation.manual_transcript {
        transcript(&evidence.cases);
    }
    let written = write_evidence(&invocation.evidence, &evidence, invocation.append);
    summary(&json!({
        "result": evidence.result,
        "scenario": evidence.scenario,
        "case": evidence.case,
        "totals": evidence.totals,
        "error": evidence.error,
        "evidence": invocation.evidence,
        "evidence_written": written.is_ok(),
        "evidence_error": written.as_ref().err().map(top_failure),
    }));
    if failed || written.is_err() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// The run fails on any top-level error, no case at all, any failed case, or, with
/// `--require-no-skips`, any skipped case.
const fn failed(error: Option<&Failure>, totals: &Totals, require_no_skips: bool) -> bool {
    error.is_some()
        || totals.cases.0 == 0
        || totals.failed.0 > 0
        || (require_no_skips && totals.skipped.0 > 0)
}

struct Prepared {
    binary: BinaryRecord,
    cases: Vec<CaseReport>,
    inventory: Option<Inventory>,
}

async fn prepare(invocation: &Invocation) -> Result<Prepared, RunnerError> {
    let binary = binary_record(&invocation.binary)?;
    let runner = std::env::current_exe().map_err(|source| RunnerError::Io {
        operation: "locate the runner executable",
        source,
    })?;
    let repo = invocation
        .scenario_root
        .as_ref()
        .map_or_else(Repo::of_build, |dir| {
            Repo::of_build().with_scenarios(dir.clone())
        });
    let ctx = Context {
        repo,
        binary: invocation.binary.clone(),
        runner,
        inject: invocation.inject.clone(),
        fixtures: invocation.fixtures.clone(),
    };
    let planned = plan(&ctx.repo, &invocation.scenario, invocation.case)?;
    let mut cases = Vec::with_capacity(planned.cases.len());
    for case in &planned.cases {
        cases.push(run_case(&ctx, case).await);
    }
    Ok(Prepared {
        binary,
        cases,
        inventory: planned.inventory,
    })
}

#[cfg(test)]
mod tests;

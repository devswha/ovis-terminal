//! One runner invocation: plan, run every case, write the evidence, choose the exit status.

use std::io::Write as _;
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
}

impl Invocation {
    fn from_cli(cli: Cli) -> Result<Self, RunnerError> {
        let missing = |detail| RunnerError::Usage { detail };
        let binary = cli.binary.ok_or_else(|| missing("--binary is required"))?;
        let scenario = cli
            .scenario
            .ok_or_else(|| missing("--scenario is required"))?;
        let evidence = cli
            .evidence
            .ok_or_else(|| missing("--evidence is required"))?;
        if !cli.isolated {
            return Err(RunnerError::IsolationRequired);
        }
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
        })
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
    let (binary, cases, error) = match prepare(&invocation).await {
        Ok((binary, cases)) => (Some(binary), cases, None),
        Err(error) => (None, Vec::new(), Some(top_failure(&error))),
    };
    let totals = Totals::of(&cases);
    let failed = error.is_some()
        || cases.is_empty()
        || totals.failed.0 > 0
        || (invocation.require_no_skips && totals.skipped.0 > 0);
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
        elapsed_ms: Decimal(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
        cases,
    };
    if invocation.manual_transcript {
        transcript(&evidence.cases);
    }
    let written = write_evidence(&invocation.evidence, &evidence);
    summary(&json!({
        "result": evidence.result,
        "scenario": evidence.scenario,
        "case": evidence.case,
        "totals": evidence.totals,
        "error": evidence.error,
        "evidence": invocation.evidence,
        "evidence_written": written.as_ref().err().map(top_failure),
    }));
    if failed || written.is_err() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

async fn prepare(invocation: &Invocation) -> Result<(BinaryRecord, Vec<CaseReport>), RunnerError> {
    let binary = binary_record(&invocation.binary)?;
    let runner = std::env::current_exe().map_err(|source| RunnerError::Io {
        operation: "locate the runner executable",
        source,
    })?;
    let ctx = Context {
        repo: Repo::of_build(),
        binary: invocation.binary.clone(),
        runner,
    };
    let planned = plan(&ctx.repo, &invocation.scenario, invocation.case)?;
    let mut cases = Vec::with_capacity(planned.len());
    for case in &planned {
        cases.push(run_case(&ctx, case).await);
    }
    Ok((binary, cases))
}

fn write_evidence(path: &std::path::Path, evidence: &Evidence) -> Result<(), RunnerError> {
    let io = |source| RunnerError::Io {
        operation: "write the evidence file",
        source,
    };
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    let bytes = serde_json::to_vec_pretty(evidence)?;
    std::fs::write(path, bytes).map_err(io)
}

fn summary(value: &serde_json::Value) {
    let mut stdout = std::io::stdout().lock();
    if writeln!(stdout, "{value}").is_err() {
        drop(writeln!(
            std::io::stderr().lock(),
            "conformance: stdout is closed"
        ));
    }
}

fn transcript(cases: &[CaseReport]) {
    let mut stderr = std::io::stderr().lock();
    for case in cases {
        for conn in &case.connections {
            for record in &conn.transcript {
                let direction = match record.direction {
                    crate::evidence::Direction::Sent => ">>",
                    crate::evidence::Direction::Received => "<<",
                };
                let text = record.text.as_deref().unwrap_or("<elided: see sha256>");
                let line = writeln!(
                    stderr,
                    "{}/{} {} {direction} @{} {}B {}: {}",
                    case.scenario,
                    case.case.as_str(),
                    conn.name,
                    record.offset.0,
                    record.bytes.0,
                    record.unit,
                    text.trim_end()
                );
                if line.is_err() {
                    return;
                }
            }
        }
    }
}

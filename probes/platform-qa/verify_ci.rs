//! `verify-ci`: prove a CI job passed every named smoke check at one commit.
//!
//! Finds the latest completed run of the workflow at the commit, requires the job's
//! conclusion to be `success`, downloads its `windows-smoke` artifact and verifies
//! the report names that commit and passes each `--require`d check. Every outcome,
//! pass or fail, is appended to the evidence file.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::Duration;

use clap::Parser;
use ovis_terminal_probes::platform_qa::{QaError, SmokeReport, append, run};
use serde_json::{Map, Value, json};

const GH_DEADLINE: Duration = Duration::from_secs(300);
const ARTIFACT: &str = "windows-smoke";

/// Verifies a CI job's smoke report at a commit and records the evidence.
#[derive(Debug, Parser)]
struct Cli {
    /// `owner/name` of the repository.
    #[arg(long)]
    repo: String,
    /// Workflow file name, such as `platform-qa.yml`.
    #[arg(long)]
    workflow: String,
    /// Job name inside the workflow.
    #[arg(long)]
    job: String,
    /// Full commit SHA the run must have tested.
    #[arg(long)]
    sha: String,
    /// Comma-separated check names that must pass.
    #[arg(long, value_delimiter = ',', required = true)]
    require: Vec<String>,
    /// Directory the artifact is downloaded into.
    #[arg(long)]
    download: PathBuf,
    /// Evidence file the outcome is appended to.
    #[arg(long)]
    evidence: PathBuf,
}

fn gh(args: &[&str]) -> Result<Vec<u8>, QaError> {
    let done = run(Command::new("gh").args(args), GH_DEADLINE)?;
    if done.exit_code == Some(0) {
        return Ok(done.stdout);
    }
    let mut command_line = vec!["gh".to_owned()];
    command_line.extend(args.iter().map(|arg| (*arg).to_owned()));
    Err(QaError::Gh {
        argv: command_line,
        exit_code: done.exit_code,
        stderr: String::from_utf8_lossy(&done.stderr).into_owned(),
    })
}

fn parse(bytes: &[u8], what: &str) -> Result<Value, QaError> {
    serde_json::from_slice(bytes).map_err(|source| QaError::Json {
        what: what.to_owned(),
        source,
    })
}

fn latest_run(cli: &Cli) -> Result<(u64, String), QaError> {
    let listed = gh(&[
        "run",
        "list",
        "--repo",
        &cli.repo,
        "--workflow",
        &cli.workflow,
        "--commit",
        &cli.sha,
        "--status",
        "completed",
        "--limit",
        "20",
        "--json",
        "databaseId,headSha,url",
    ])?;
    let runs = parse(&listed, "gh run list")?;
    runs.as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry.get("headSha").and_then(Value::as_str) == Some(&cli.sha))
        .find_map(|entry| {
            let id = entry.get("databaseId")?.as_u64()?;
            let url = entry.get("url")?.as_str()?.to_owned();
            Some((id, url))
        })
        .ok_or_else(|| QaError::RunNotFound {
            workflow: cli.workflow.clone(),
            sha: cli.sha.clone(),
        })
}

fn job_conclusion(cli: &Cli, run_id: &str) -> Result<String, QaError> {
    let viewed = gh(&["run", "view", run_id, "--repo", &cli.repo, "--json", "jobs"])?;
    let jobs = parse(&viewed, "gh run view")?;
    let conclusion = jobs
        .get("jobs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|job| job.get("name").and_then(Value::as_str) == Some(&cli.job))
        .and_then(|job| job.get("conclusion")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| "missing".to_owned());
    if conclusion == "success" {
        Ok(conclusion)
    } else {
        Err(QaError::JobNotGreen {
            job: cli.job.clone(),
            conclusion,
        })
    }
}

fn downloaded_report(cli: &Cli, run_id: &str) -> Result<SmokeReport, QaError> {
    let dir = cli.download.to_string_lossy().into_owned();
    drop(std::fs::remove_dir_all(&cli.download));
    gh(&[
        "run", "download", run_id, "--repo", &cli.repo, "--name", ARTIFACT, "--dir", &dir,
    ])?;
    let path = cli.download.join("windows-smoke.json");
    let bytes = std::fs::read(&path).map_err(|source| QaError::Io {
        operation: "read the downloaded smoke report",
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| QaError::Json {
        what: path.display().to_string(),
        source,
    })
}

type Record = Map<String, Value>;

fn put(record: &mut Record, (key, value): (&str, Value)) {
    record.insert(key.to_owned(), value);
}

fn verify(cli: &Cli, record: &mut Record) -> Result<(), QaError> {
    let (run_id, url) = latest_run(cli)?;
    let run_id = run_id.to_string();
    put(record, ("run_id", json!(run_id)));
    put(record, ("run_url", json!(url)));
    put(
        record,
        ("job_conclusion", json!(job_conclusion(cli, &run_id)?)),
    );
    let report = downloaded_report(cli, &run_id)?;
    let checks: Vec<Value> = report
        .checks
        .iter()
        .map(
            |check| json!({"name": check.name, "status": check.status, "observes": check.observes}),
        )
        .collect();
    let summary = json!({
        "windows_smoke": report.windows_smoke, "os": report.os, "arch": report.arch,
        "scope": report.scope, "sha": report.sha, "checks": checks,
    });
    put(record, ("report", summary));
    report.verify(&cli.sha, &cli.require)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut record = Record::new();
    for (key, value) in [
        ("format", json!("ovis-ci-verification/1")),
        ("repo", json!(cli.repo)),
        ("workflow", json!(cli.workflow)),
        ("job", json!(cli.job)),
        ("sha", json!(cli.sha)),
        ("required", json!(cli.require)),
    ] {
        put(&mut record, (key, value));
    }
    let outcome = verify(&cli, &mut record);
    put(
        &mut record,
        (
            "result",
            json!(if outcome.is_ok() { "passed" } else { "failed" }),
        ),
    );
    if let Err(error) = &outcome {
        let failure =
            json!({"kind": error.kind(), "hint": error.hint(), "detail": error.to_string()});
        put(&mut record, ("error", failure));
    }
    let written = append(&cli.evidence, Value::Object(record.clone()));
    if let Err(error) = &written {
        let failure = json!({"kind": error.kind(), "detail": error.to_string()});
        put(&mut record, ("evidence_written", failure));
    }
    drop(writeln!(
        std::io::stdout().lock(),
        "{}",
        Value::Object(record)
    ));
    if outcome.is_ok() && written.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

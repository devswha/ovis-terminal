//! `windows-smoke`: run every check of the smoke manifest and write the report.
//!
//! Each native test runs alone (`cargo test <path> -- --exact`) under a deadline; a
//! check passes only when each of its tests ran exactly once and passed, so a
//! missing prerequisite or an ignored test fails the check instead of skipping it.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

use clap::Parser;
use ovis_terminal_probes::platform_qa::{
    Check, CheckReport, NativeTest, QaError, SmokeManifest, SmokeReport, TestRun, run,
};

/// Runs the Windows contract smoke checks and writes `windows-smoke.json`.
#[derive(Debug, Parser)]
struct Cli {
    /// The smoke manifest.
    #[arg(long)]
    manifest: PathBuf,
    /// Directory for the report and one log per test run.
    #[arg(long)]
    out: PathBuf,
    /// The commit under test, recorded in the report.
    #[arg(long)]
    sha: String,
    /// Deadline of one test run in seconds.
    #[arg(long, default_value_t = 900)]
    deadline_secs: u64,
}

fn run_test(test: &NativeTest, (cli, log): (&Cli, &str)) -> Result<TestRun, QaError> {
    let done = run(
        Command::new("cargo").args(test.cargo_args()),
        Duration::from_secs(cli.deadline_secs),
    )?;
    let record = TestRun::captured(test, &done, log);
    let mut transcript = format!(
        "$ {}\nexit_code: {:?}\ntimed_out: {}\nverdict: {:?} {:?}\n--- stdout ---\n",
        record.argv.join(" "),
        done.exit_code,
        done.timed_out,
        record.verdict,
        record.failure
    )
    .into_bytes();
    transcript.extend_from_slice(&done.stdout);
    transcript.extend_from_slice(b"\n--- stderr ---\n");
    transcript.extend_from_slice(&done.stderr);
    std::fs::write(cli.out.join(log), transcript).map_err(|source| QaError::Io {
        operation: "write a test log",
        source,
    })?;
    Ok(record)
}

fn run_check(check: &Check, cli: &Cli) -> Result<CheckReport, QaError> {
    let mut tests = Vec::with_capacity(check.tests.len());
    for (index, test) in check.tests.iter().enumerate() {
        let log = format!("{}-{index}.log", check.name);
        tests.push(run_test(test, (cli, &log))?);
    }
    Ok(CheckReport::of(check, tests))
}

fn write_report(report: &SmokeReport, out: &Path) -> Result<(), QaError> {
    let bytes = serde_json::to_vec_pretty(report).map_err(|source| QaError::Json {
        what: "the smoke report".to_owned(),
        source,
    })?;
    std::fs::write(out.join("windows-smoke.json"), bytes).map_err(|source| QaError::Io {
        operation: "write the smoke report",
        source,
    })
}

fn smoke(cli: &Cli) -> Result<SmokeReport, QaError> {
    let manifest = SmokeManifest::load(&cli.manifest)?;
    std::fs::create_dir_all(&cli.out).map_err(|source| QaError::Io {
        operation: "create the output directory",
        source,
    })?;
    let mut checks = Vec::with_capacity(manifest.checks.len());
    for check in &manifest.checks {
        checks.push(run_check(check, cli)?);
    }
    let report = SmokeReport::assemble(&cli.sha, checks);
    write_report(&report, &cli.out)?;
    Ok(report)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let outcome = smoke(&cli).and_then(|report| {
        let failed = report.failed();
        let lines: Vec<String> = report
            .checks
            .iter()
            .map(|check| format!("{}: {:?}", check.name, check.status))
            .collect();
        drop(writeln!(std::io::stdout().lock(), "{}", lines.join("\n")));
        if failed == 0 {
            Ok(())
        } else {
            Err(QaError::SmokeFailed { failed })
        }
    });
    let summary = match &outcome {
        Ok(()) => serde_json::json!({"windows_smoke": "pass"}),
        Err(error) => serde_json::json!({
            "windows_smoke": "fail", "kind": error.kind(), "hint": error.hint(),
            "detail": error.to_string(),
        }),
    };
    drop(writeln!(std::io::stdout().lock(), "{summary}"));
    if outcome.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

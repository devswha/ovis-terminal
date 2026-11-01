//! The fifty-terminal benchmark of plan todo 69 (IDEAL-027, IDEAL-041, IDEAL-051,
//! IDEAL-116).
//!
//! `terminal50 --binary <ovis-terminal> --isolated --terminals N --clients C --inflight F
//! --output-bytes-per-second R --samples S` plus exactly one budget pair: the two max
//! bounds judge the fixed IDEAL shape on every sample; the two p99 bounds judge a
//! separately labelled capacity release target. The report goes to stdout and to
//! `--evidence`, raw samples beside it; a broken budget exits 1 with `budget_exceeded`.

mod account;
mod cli;
mod delivery;
mod drive;
mod emit;
mod error;
mod fleet;
mod host;
mod lanes;
mod measure;
mod plan;
mod pty;
mod report;
mod settle;
mod stats;
#[cfg(test)]
mod tests;
mod verdict;

use std::ffi::OsString;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser as _;
pub use error::Terminal50Error;

fn finish(stdout: &[u8], failure: Option<&Terminal50Error>) -> ExitCode {
    let mut out = io::stdout().lock();
    if !stdout.is_empty()
        && out
            .write_all(stdout)
            .and_then(|()| out.write_all(b"\n"))
            .is_err()
    {
        return ExitCode::from(3);
    }
    let Some(error) = failure else {
        return ExitCode::SUCCESS;
    };
    let line = format!("{}: {error} ({})\n", error.kind(), error.hint());
    if io::stderr().write_all(line.as_bytes()).is_err() {
        return ExitCode::from(3);
    }
    ExitCode::from(error.exit_code())
}

fn write_evidence(path: &Path, report: &[u8], raw: &account::Raw) -> Result<(), Terminal50Error> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, report)?;
    std::fs::write(
        path.with_extension("samples.json"),
        serde_json::to_vec(raw)?,
    )?;
    Ok(())
}

/// The report bytes and, when a budget broke, the failure to report.
fn bench(
    cli: &cli::Cli,
    exe: PathBuf,
) -> Result<(Vec<u8>, Option<Terminal50Error>), Terminal50Error> {
    let (plan, binary) = cli.plan(exe)?;
    let binary = host::binary(&binary)?;
    if let Some(base) = plan.root.parent() {
        std::fs::create_dir_all(base)?;
    }
    std::fs::create_dir(&plan.root)?;
    let measured = drive::run(&plan, binary);
    let removed = std::fs::remove_dir_all(&plan.root);
    let (report, raw) = measured?;
    removed?;
    let json = serde_json::to_vec_pretty(&report)?;
    if let Some(path) = &cli.evidence {
        write_evidence(path, &json, &raw)?;
    }
    let failure = (!report.verdict.passed).then(|| Terminal50Error::BudgetExceeded {
        violations: report.verdict.violations.len(),
    });
    Ok((json, failure))
}

/// Runs the command line `args` (program name excluded) and returns the exit status:
/// 0 when every budget held, 1 for `budget_exceeded` or a failed run, 2 for arguments.
#[must_use]
pub fn main_with(args: Vec<OsString>) -> ExitCode {
    let command_line = std::iter::once(OsString::from("terminal50")).chain(args);
    let cli = match cli::Cli::try_parse_from(command_line) {
        Ok(cli) => cli,
        Err(error) => {
            let code = u8::try_from(error.exit_code()).unwrap_or(2);
            return if error.print().is_ok() {
                ExitCode::from(code)
            } else {
                ExitCode::from(3)
            };
        }
    };
    if let Some(cli::Mode::Emit { bytes_per_second }) = cli.mode {
        return if emit::run(bytes_per_second).is_ok() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    match std::env::current_exe()
        .map_err(Terminal50Error::from)
        .and_then(|exe| bench(&cli, exe))
    {
        Ok((json, failure)) => finish(&json, failure.as_ref()),
        Err(error) => finish(&[], Some(&error)),
    }
}

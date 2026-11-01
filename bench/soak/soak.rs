//! The continuous soak of plan todo 70 (IDEAL-041, IDEAL-043, IDEAL-092..096, IDEAL-116,
//! IDEAL-117).
//!
//! `soak --binary <ovis-terminal> --isolated --terminals N --duration-seconds S
//! --core-crashes C ...` runs N programs in the holders of the binary under test (its
//! own `service run`, one shared holder plus one dedicated holder for the last terminal)
//! for S continuous seconds, while a control process that owns every screen is crashed
//! with SIGKILL, handed over, observed through interruptions and stalled viewers. It
//! samples memory and disk every second into durable raw files and judges unchanged
//! program identities, output offsets, bounded queues, control availability and memory
//! growth. `--inject '{"fixture":"holder-tail-gap-and-lost-receipt"}'` runs the failure
//! fixture instead. Exit 0 only when every rule held; 1 otherwise; 2 for arguments.

mod account;
mod actions;
mod cli;
mod control;
mod drive;
mod emit;
mod error;
mod fixture;
mod holders;
mod host;
mod launch;
mod observer;
mod plan;
mod process;
mod report;
mod samples;
#[cfg(test)]
mod tests;
mod verdict;
mod wire;

use std::ffi::OsString;
use std::io::{self, Write as _};
use std::path::Path;
use std::process::ExitCode;

use clap::Parser as _;
pub use error::SoakError;

fn fail(error: &SoakError) -> ExitCode {
    let line = format!("{}: {error} ({})\n", error.kind(), error.hint());
    if io::stderr().write_all(line.as_bytes()).is_err() {
        return ExitCode::from(3);
    }
    ExitCode::from(error.exit_code())
}

fn write_evidence(path: &Path, report: &[u8]) -> Result<(), SoakError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(SoakError::io("create the evidence directory"))?;
    }
    std::fs::write(path, report).map_err(SoakError::io("write the evidence"))
}

/// Runs the harness; the report bytes and the number of broken rules.
fn soak(cli: &cli::Cli) -> Result<(Vec<u8>, usize), SoakError> {
    let (plan, binary) = cli.plan()?;
    let facts = host::binary(&binary)?;
    let exe = std::env::current_exe().map_err(SoakError::io("locate this executable"))?;
    let evidence = plan
        .evidence
        .clone()
        .unwrap_or_else(|| plan.root.with_extension("json"));
    let mut run = drive::Run::new(&plan, (exe, &binary), &evidence)?;
    let mut outcome = report::Outcome::default();
    let measured = run.start_holders().and_then(|()| {
        if plan.fixture.is_some() {
            outcome.fixture = Some(run.fixture()?);
            return Ok(());
        }
        outcome.elapsed = run.measure()?;
        if plan.holder_loss_probe {
            outcome.probe = Some(run.probe()?);
        }
        Ok(())
    });
    outcome.error = measured.err();
    outcome.elapsed = outcome.elapsed.max(run.elapsed);
    outcome.teardown = run.teardown().err();
    let (value, violations) = report::report(&run, (&facts, &outcome));
    let json = serde_json::to_vec_pretty(&value)?;
    write_evidence(&evidence, &json)?;
    Ok((json, violations))
}

/// Runs the command line `args` (program name excluded) and returns the exit status.
#[must_use]
pub fn main_with(args: Vec<OsString>) -> ExitCode {
    let command_line = std::iter::once(OsString::from("soak")).chain(args);
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
    match &cli.mode {
        Some(cli::Mode::Control {
            root,
            boot_mode,
            boot,
        }) => {
            return control::run(root, (*boot_mode, *boot))
                .map_or_else(|error| fail(&error), |()| ExitCode::SUCCESS);
        }
        Some(cli::Mode::Emit { bytes_per_second }) => {
            return if emit::run(*bytes_per_second).is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
        None => {}
    }
    match soak(&cli) {
        Ok((json, violations)) => {
            let mut out = io::stdout().lock();
            if out
                .write_all(&json)
                .and_then(|()| out.write_all(b"\n"))
                .is_err()
            {
                return ExitCode::from(3);
            }
            if violations == 0 {
                ExitCode::SUCCESS
            } else {
                fail(&SoakError::Failed { violations })
            }
        }
        Err(error) => fail(&error),
    }
}

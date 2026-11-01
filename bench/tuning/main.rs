//! `fairness-tuning` prints one JSON report of the plan todo 56 scheduler tuning harness;
//! see `bench/tuning/tuning.rs` for its modes. Unix only: Windows stays at the CI smoke tier.

use std::io::{self, Write as _};
use std::process::ExitCode;

#[cfg(unix)]
fn main() -> ExitCode {
    use ovis_terminal_bench::tuning::{TuningError, run};

    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(json) if json.is_empty() => ExitCode::SUCCESS,
        Ok(json) => {
            let mut stdout = io::stdout().lock();
            let written = stdout
                .write_all(&json)
                .and_then(|()| stdout.write_all(b"\n"));
            if written.is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(3)
            }
        }
        Err(error) => {
            let line = format!("{}: {error} ({})\n", error.kind(), error.hint());
            if io::stderr().write_all(line.as_bytes()).is_err() {
                return ExitCode::from(3);
            }
            ExitCode::from(if matches!(error, TuningError::Usage) {
                2
            } else {
                1
            })
        }
    }
}

#[cfg(not(unix))]
fn main() -> ExitCode {
    let line = b"usage: fairness-tuning runs on Unix hosts only\n";
    if io::stderr().write_all(line).is_err() {
        return ExitCode::from(3);
    }
    ExitCode::from(2)
}

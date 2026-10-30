//! `pty-harness latency` runs the local latency profile and
//! `pty-harness soak <terminals> <seconds>` runs the capacity soak; each prints
//! one JSON report on stdout. Unix only: Windows stays at the CI smoke tier.

use std::io::{self, Write};
use std::process::ExitCode;

use thiserror::Error;

#[derive(Debug, Error)]
enum CliError {
    #[error("expected `latency` or `soak <terminals> <seconds>`")]
    Usage,
    #[cfg(unix)]
    #[error("the harness run failed")]
    Harness(#[from] ovis_terminal_bench::harness::HarnessError),
    #[error("the report could not be written")]
    Output(#[from] io::Error),
    #[error("the report could not be encoded")]
    Encode(#[from] serde_json::Error),
}

impl CliError {
    const fn kind(&self) -> &'static str {
        match self {
            Self::Usage => "usage",
            #[cfg(unix)]
            Self::Harness(source) => source.kind(),
            Self::Output(_) => "output_failed",
            Self::Encode(_) => "encode_failed",
        }
    }

    const fn hint(&self) -> &'static str {
        match self {
            Self::Usage => "pass `latency`, or `soak` with a terminal count and seconds",
            #[cfg(unix)]
            Self::Harness(source) => source.hint(),
            Self::Output(_) | Self::Encode(_) => "check that stdout is writable",
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = report(&args).and_then(|json| {
        let mut stdout = io::stdout().lock();
        stdout.write_all(&json)?;
        stdout.write_all(b"\n")?;
        Ok(stdout.flush()?)
    });
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let line = format!("{}: {error} ({})\n", error.kind(), error.hint());
            if io::stderr().write_all(line.as_bytes()).is_err() {
                return ExitCode::from(3);
            }
            ExitCode::from(if matches!(error, CliError::Usage) {
                2
            } else {
                1
            })
        }
    }
}

#[cfg(unix)]
fn report(args: &[String]) -> Result<Vec<u8>, CliError> {
    use std::time::Duration;

    use ovis_terminal_bench::harness::{LatencyProfile, SoakProfile, run_latency, run_soak};

    match args {
        [mode] if mode == "latency" => Ok(serde_json::to_vec_pretty(&run_latency(
            &LatencyProfile::LOCAL,
        )?)?),
        [mode, terminals, seconds] if mode == "soak" => {
            let terminals = terminals.parse().map_err(|_not_a_count| CliError::Usage)?;
            let seconds = seconds.parse().map_err(|_not_a_count| CliError::Usage)?;
            let profile = SoakProfile::new(terminals, Duration::from_secs(seconds));
            Ok(serde_json::to_vec_pretty(&run_soak(&profile)?)?)
        }
        _ => Err(CliError::Usage),
    }
}

#[cfg(not(unix))]
const fn report(_args: &[String]) -> Result<Vec<u8>, CliError> {
    Err(CliError::Usage)
}

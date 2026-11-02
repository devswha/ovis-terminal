//! `release-stage`: prints one JSON report on stdout and exits with its status.

use std::io::Write as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let run = release_stage::execute(&arguments);
    let written = serde_json::to_string_pretty(&run.report)
        .map_err(std::io::Error::other)
        .and_then(|text| writeln!(std::io::stdout().lock(), "{text}"));
    match written {
        Ok(()) => ExitCode::from(run.exit),
        Err(_) => ExitCode::FAILURE,
    }
}

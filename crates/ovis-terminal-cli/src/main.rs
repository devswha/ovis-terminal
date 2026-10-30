//! `ovis-terminal` binary entry point.

use std::ffi::OsString;
use std::io::{self, Write as _};
use std::process::ExitCode;

use ovis_terminal_cli::{Invocation, USAGE_EXIT_CODE, version_line};

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match Invocation::parse(&args) {
        Ok(Invocation::Version) => match writeln!(io::stdout().lock(), "{}", version_line()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        },
        Err(error) => match writeln!(io::stderr().lock(), "ovis-terminal: {error}") {
            Ok(()) => ExitCode::from(USAGE_EXIT_CODE),
            Err(_) => ExitCode::FAILURE,
        },
    }
}

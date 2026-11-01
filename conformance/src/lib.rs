//! Real-socket conformance runner for the `ovis-terminal` binary.
//!
//! Each case runs in a fresh isolated root: the runner starts its fixture process (the
//! real server crate, dispatcher and store behind a private socket), connects through the
//! binary's `connect` bridge or its own socket, records every byte and checks typed
//! expectations. Exit 0 only when every planned case passed with at least one assertion;
//! 1 on any failure, missing scenario, timeout or unwritable evidence; 2 on usage errors.

pub mod cli;
pub mod engine;
pub mod error;
pub mod evidence;
pub mod fixture;
pub mod ids;
pub mod invocation;
pub mod report;
pub mod scenario;
#[cfg(test)]
mod traceability;

use std::io::Write as _;
use std::process::ExitCode;

use clap::Parser as _;

use crate::cli::{Cli, Mode};

/// Parses the command line and runs the runner or, internally, the fixture process.
pub fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let code = if error.use_stderr() { 2 } else { 0 };
            drop(error.print());
            return ExitCode::from(code);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            drop(writeln!(
                std::io::stderr().lock(),
                "conformance: no async runtime: {error}"
            ));
            return ExitCode::FAILURE;
        }
    };
    match cli.mode {
        Some(Mode::Fixture { root, peer, binary }) => {
            match runtime.block_on(fixture::serve(&root, fixture::Role::of(peer, binary))) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    let mut chain = error.to_string();
                    let mut source = std::error::Error::source(&error);
                    while let Some(inner) = source {
                        chain.push_str(": ");
                        chain.push_str(&inner.to_string());
                        source = inner.source();
                    }
                    let line = serde_json::json!({"error": {"kind": error.kind(), "hint": error.hint(), "detail": chain}});
                    drop(writeln!(std::io::stderr().lock(), "{line}"));
                    ExitCode::FAILURE
                }
            }
        }
        None => runtime.block_on(invocation::main(cli)),
    }
}

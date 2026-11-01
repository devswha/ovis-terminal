//! `ovis-replay refresh`: re-derive the predicates and re-pin the fixture manifest.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Maintains the Ovis consumer fixture set.
#[derive(Debug, Parser)]
#[command(name = "ovis-replay", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Rewrite predicates.json from the ideal map and every pin of manifest.json.
    Refresh {
        /// Repository root.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Fixture directory.
        #[arg(long, default_value = "conformance/fixtures/ovis")]
        fixtures: PathBuf,
    },
}

fn main() -> ExitCode {
    let Command::Refresh { repo, fixtures } = Cli::parse().command;
    let scenarios = repo.join("conformance").join("scenarios");
    let absent = |id: &str| !scenarios.join(format!("{id}.json")).is_file();
    let outcome = ovis_terminal_replay::refresh(&repo, (&fixtures, &absent));
    let line = match &outcome {
        Ok(manifest) => serde_json::json!({"refreshed": manifest.files.len()}),
        Err(error) => {
            serde_json::json!({"error": {"kind": error.kind(), "hint": error.hint(), "detail": error.to_string()}})
        }
    };
    if writeln!(std::io::stdout().lock(), "{line}").is_err() {
        return ExitCode::FAILURE;
    }
    if outcome.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

//! The runner's command line.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// `conformance --binary <path> --scenario <family/name> --isolated --case <c> --evidence <file>`.
#[derive(Debug, Parser)]
#[command(
    name = "conformance",
    version,
    about = "Drive the real ovis-terminal binary over a real socket and record typed evidence.",
    args_conflicts_with_subcommands = true,
    disable_help_subcommand = true
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) mode: Option<Mode>,
    /// The ovis-terminal binary under test.
    #[arg(long)]
    pub(crate) binary: Option<PathBuf>,
    /// Scenario identity such as `protocol/multiplex`, or `all/all`.
    #[arg(long)]
    pub(crate) scenario: Option<String>,
    /// Run in a fresh isolated session root; required, the runner never uses a live session.
    #[arg(long)]
    pub(crate) isolated: bool,
    /// Which declared cases to run.
    #[arg(long, value_enum, default_value_t = CaseSelector::All)]
    pub(crate) case: CaseSelector,
    /// Where to write the evidence JSON.
    #[arg(long)]
    pub(crate) evidence: Option<PathBuf>,
    /// Fail when any case is skipped, for example on an undeclared platform.
    #[arg(long)]
    pub(crate) require_no_skips: bool,
    /// Also print every connection transcript to stderr for manual inspection.
    #[arg(long)]
    pub(crate) manual_transcript: bool,
    /// Read scenarios from this directory instead of `conformance/scenarios`.
    #[arg(long)]
    pub(crate) scenario_root: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Mode {
    /// Serve the protocol fixture in <root> until stdin closes (started by the runner).
    #[command(hide = true)]
    Fixture {
        #[arg(long)]
        root: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum CaseSelector {
    Happy,
    Failure,
    All,
}

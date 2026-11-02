//! The `release-stage` command line.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Stages, seals, natively checks and promotes ovis-terminal release artifacts. Nothing
/// here uploads; `promote` only decides whether the workflow may publish.
#[derive(Debug, Parser)]
#[command(name = "release-stage", disable_help_subcommand = true)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Step,
}

/// One release step.
#[derive(Debug, Subcommand)]
pub(crate) enum Step {
    /// Observe the build host and toolchain into a receipt.
    Receipt {
        #[arg(long)]
        target: String,
        #[arg(long, default_value = "local")]
        runner: String,
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// `name=value` C toolchain observation; repeatable.
        #[arg(long = "observe")]
        observed: Vec<String>,
    },
    /// Check a built binary against the source tree and lay out its artifact.
    Stage {
        #[arg(long)]
        target: String,
        #[arg(long)]
        binary: PathBuf,
        #[arg(long)]
        sdk: PathBuf,
        #[arg(long)]
        receipt: PathBuf,
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        released: Option<PathBuf>,
        #[arg(long)]
        notices: Option<PathBuf>,
    },
    /// Pack a staged artifact and record its archive digest.
    Seal {
        #[arg(long)]
        entry: PathBuf,
    },
    /// Unpack and run a sealed artifact on its own native host.
    Qa {
        #[arg(long)]
        entry: PathBuf,
        #[arg(long)]
        install_dir: PathBuf,
        #[arg(long)]
        released: PathBuf,
        #[arg(long, default_value = "local")]
        runner: String,
    },
    /// Re-hash every staged file and archive of a directory.
    Verify {
        #[arg(long)]
        dir: PathBuf,
    },
    /// Gate all five native targets and decide whether to publish.
    Promote {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        version: String,
        #[arg(long)]
        pins: PathBuf,
        #[arg(long)]
        existing: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Regenerate (or `--check`) the crate inventory of `THIRD_PARTY_NOTICES.md`.
    Notices {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        check: bool,
    },
    /// Run one seeded conformance fixture in a scratch directory.
    Fixture {
        name: String,
        #[arg(long)]
        binary: PathBuf,
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        scratch: PathBuf,
    },
}

//! The command lines of the five quality tools.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

/// `source-check`: source rules, the strict Cargo config and the checklist.
#[derive(Debug, Parser)]
#[command(
    name = "source-check",
    about = "Syntax-aware source rules and the violations checklist"
)]
pub(super) struct SourceCheckArgs {
    /// Directories whose `.rs` files are checked, relative to --repo.
    #[arg(long, num_args = 1.., required_unless_present = "fixture")]
    pub(super) roots: Vec<PathBuf>,
    #[arg(long, default_value_t = 250)]
    pub(super) max_pure_loc: usize,
    #[arg(long, default_value_t = 3)]
    pub(super) max_params: usize,
    #[arg(long, default_value = "docs/research/violations.md")]
    pub(super) violations: PathBuf,
    #[arg(long, default_value = "docs/quality-checklist.md")]
    pub(super) checklist: PathBuf,
    /// Write the regenerated checklist before comparing it.
    #[arg(long)]
    pub(super) write_checklist: bool,
    #[arg(long, default_value = ".")]
    pub(super) repo: PathBuf,
    /// Check one seeded source instead of the roots.
    #[arg(long)]
    pub(super) fixture: Option<String>,
    /// Append this run's report to an evidence file.
    #[arg(long)]
    pub(super) evidence: Option<PathBuf>,
}

/// `evidence-audit`: conformance evidence documents.
#[derive(Debug, Parser)]
#[command(
    name = "evidence-audit",
    about = "Reject pass claims that the evidence does not prove"
)]
pub(super) struct EvidenceAuditArgs {
    /// Evidence documents written by the conformance runner.
    #[arg(long = "input", num_args = 1.., required_unless_present = "fixture")]
    pub(super) inputs: Vec<PathBuf>,
    #[arg(long)]
    pub(super) fixture: Option<String>,
    #[arg(long)]
    pub(super) evidence: Option<PathBuf>,
}

/// `plan-audit`: the F1 plan compliance gate.
#[derive(Debug, Parser)]
#[command(
    name = "plan-audit",
    about = "Coverage, references, QA scenarios and write scopes of the plan"
)]
pub(super) struct PlanAuditArgs {
    #[arg(long, required_unless_present = "fixture")]
    pub(super) plan: Option<PathBuf>,
    #[arg(long, default_value = "docs/design/ideal-state.md")]
    pub(super) ideals: PathBuf,
    #[arg(long, default_value = "docs/design/gap-list.md")]
    pub(super) gaps: PathBuf,
    #[arg(long, default_value = ".")]
    pub(super) scope_root: PathBuf,
    #[arg(long)]
    pub(super) deny_write_root: Vec<PathBuf>,
    /// Inclusive todo range such as `1-70`.
    #[arg(long, default_value = "1-70")]
    pub(super) require_todos: String,
    #[arg(long, default_value_t = 117)]
    pub(super) require_ideals: u16,
    #[arg(long, default_value_t = 150)]
    pub(super) require_gaps: u16,
    #[arg(long, default_value_t = 27)]
    pub(super) require_exclusions: u16,
    /// Fail on unresolved references instead of only listing them.
    #[arg(long)]
    pub(super) reject_unresolved: bool,
    #[arg(long)]
    pub(super) fixture: Option<String>,
    #[arg(long)]
    pub(super) evidence: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum PlatformArg {
    Linux,
    Macos,
    Windows,
}

impl PlatformArg {
    pub(super) const fn os(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
        }
    }
}

/// `ideal-audit`: the F4 ideal-state fidelity gate.
#[derive(Debug, Parser)]
#[command(
    name = "ideal-audit",
    about = "Prove every IDEAL row from audited evidence"
)]
pub(super) struct IdealAuditArgs {
    #[arg(long, default_value = "docs/design/ideal-state.md")]
    pub(super) ideals: PathBuf,
    #[arg(long, default_value = "docs/design/gap-list.md")]
    pub(super) gaps: PathBuf,
    #[arg(long, default_value = "conformance/ideal-map.json")]
    pub(super) mapping: PathBuf,
    /// Directory searched recursively for evidence and soak receipts.
    #[arg(long, required_unless_present = "fixture")]
    pub(super) evidence_dir: Option<PathBuf>,
    #[arg(long, default_value_t = 117)]
    pub(super) require_rows: u16,
    #[arg(long = "require-platform", value_enum)]
    pub(super) require_platforms: Vec<PlatformArg>,
    #[arg(long)]
    pub(super) windows_contract_smoke: bool,
    #[arg(long, default_value_t = 0)]
    pub(super) require_continuous_soak_seconds: u64,
    /// Append this run's report to this file.
    #[arg(long)]
    pub(super) report: Option<PathBuf>,
    #[arg(long)]
    pub(super) fixture: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum MatrixMode {
    /// Regenerate both files from the plan.
    Write,
    /// Regenerate in memory and fail on any difference.
    Check,
    /// Structural check without the plan: ledger coverage, predicates, schema contract.
    Verify,
}

/// `quality-matrix`: the traceability matrix and the IDEAL map.
#[derive(Debug, Parser)]
#[command(
    name = "quality-matrix",
    about = "Generate or check conformance/traceability.json and ideal-map.json"
)]
pub(super) struct MatrixArgs {
    #[arg(long, value_enum)]
    pub(super) mode: MatrixMode,
    /// The work plan; required by write and check.
    #[arg(long)]
    pub(super) plan: Option<PathBuf>,
    /// How the plan is named inside the generated files.
    #[arg(long, default_value = ".omo/plans/ovis-terminal-v1.md")]
    pub(super) plan_label: String,
    #[arg(long, default_value = "docs/design/ideal-state.md")]
    pub(super) ideals: PathBuf,
    #[arg(long, default_value = "docs/design/gap-list.md")]
    pub(super) gaps: PathBuf,
    #[arg(long, default_value = "schema/ovis-terminal.openrpc.json")]
    pub(super) schema: PathBuf,
    #[arg(long, default_value = "conformance/traceability.json")]
    pub(super) traceability: PathBuf,
    #[arg(long, default_value = "conformance/ideal-map.json")]
    pub(super) ideal_map: PathBuf,
}

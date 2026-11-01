//! Quality gates for the workspace (plan todo 61).
//!
//! Five tools share this module, each a thin binary under `probes/quality/bin/`:
//!
//! - `source-check`: syntax-aware pure-LOC, parameter, dropped-result, `#[allow]`,
//!   numeric-cast and tautological-assertion rules over Rust sources, the strict Cargo
//!   lint configuration, and the review checklist generated from every rule id in
//!   `docs/research/violations.md` Rule totals.
//! - `evidence-audit`: rejects conformance evidence whose pass claim is not backed by
//!   parsed exit statuses, recomputed assertions and a binary hash.
//! - `plan-audit`: rejects a plan with uncovered IDEAL/GAP/EXCL rows, unresolved
//!   references, missing QA scenarios or overlapping same-wave write scopes.
//! - `ideal-audit`: proves every IDEAL row only from audited evidence.
//! - `quality-matrix`: regenerates or checks `conformance/traceability.json` and
//!   `conformance/ideal-map.json` from the plan, the ledgers and the OpenRPC schema.
//!
//! Every tool takes `--fixture <name>` to run against a seeded defect (or a clean
//! fixture) shipped with it, so each gate demonstrably fails on what it guards.
//! Exit status: 0 pass, 1 the gate found a defect, 2 usage error, 3 unreadable input.

mod args;
mod cargo_rules;
mod checklist;
mod cli;
mod contract;
mod discards;
mod error;
mod evidence;
mod exemptions;
mod gate_report;
mod ideal_audit;
mod ledgers;
mod markdown;
mod matchers;
mod matrix;
mod matrix_build;
mod plan;
mod plan_audit;
mod render;
mod report;
mod rules;
mod scan;
mod seeds;
mod tools_audit;
mod tools_matrix;
mod tools_source;
mod violations;

pub use cli::{GateRun, Tool, execute, main};
pub use error::QualityError;

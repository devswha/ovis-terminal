//! Release staging for ovis-terminal (plan todo 67).
//!
//! Each step is a function from closed JSON documents to closed JSON documents:
//! `receipt` observes a native build, `stage` checks the built binary against the source
//! tree (versions, schema, released-schema compatibility, notices) and lays out its
//! artifact, `seal` packs it, `qa` unpacks and runs it on its own host, and `promote`
//! refuses publication unless all five native targets carry passing receipts. The
//! conformance runner calls [`execute`] in-process, exactly as the binary would run.

pub mod binary;
pub mod build_output;
pub mod cli;
pub mod docs;
pub mod error;
pub mod files;
pub mod fixtures;
pub mod gate_fixtures;
pub mod matrix;
pub mod notices;
pub mod promote;
pub mod qa;
pub mod receipt;
pub mod seal;
pub mod stage;
pub mod target;
#[cfg(test)]
mod tests;

use clap::Parser as _;
use serde_json::{Value, json};

use self::cli::{Cli, Step};
use self::error::ReleaseError;

/// One finished run: exit status, JSON report and the argv it ran with.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct StageRun {
    /// 0 pass, 1 refused, 2 usage, 3 unreadable input or failed helper program.
    pub exit: u8,
    /// The report the binary prints on stdout.
    pub report: Value,
    /// `release-stage` followed by the arguments.
    pub argv: Vec<String>,
}

fn run(step: Step) -> Result<(u8, Value), ReleaseError> {
    let passed = |report: Value| Ok((0, report));
    match step {
        Step::Receipt {
            target,
            runner,
            repo,
            out,
            observed,
        } => {
            let made = receipt::receipt(&receipt::ReceiptInput {
                target,
                runner,
                repo,
                observed,
            })?;
            files::write_json(&out, &made)?;
            passed(
                json!({ "receipt": out, "host": made.host, "rustc": made.rustc, "sqlite": made.sqlite }),
            )
        }
        Step::Stage {
            target,
            binary,
            sdk,
            receipt,
            repo,
            out,
            released,
            notices,
        } => {
            let input = stage::StageInput {
                target,
                binary,
                sdk,
                receipt,
                repo,
                out,
                released,
                notices,
            };
            passed(stage::stage(&input)?)
        }
        Step::Seal { entry } => passed(seal::seal(&entry)?),
        Step::Qa {
            entry,
            install_dir,
            released,
            runner,
        } => passed(qa::qa(&qa::QaInput {
            entry,
            install_dir,
            released,
            runner,
        })?),
        Step::Verify { dir } => passed(seal::verify(&dir)?),
        Step::Promote {
            dir,
            version,
            pins,
            existing,
            dry_run,
        } => passed(promote::promote(&promote::PromoteInput {
            dir,
            version,
            pins,
            existing,
            dry_run,
        })?),
        Step::Notices { repo, check } => {
            let crates = notices::refresh(&repo, check)?;
            passed(json!({ "crates": crates.to_string(), "checked": check }))
        }
        Step::Fixture {
            name,
            binary,
            repo,
            scratch,
        } => fixtures::run(
            &name,
            &fixtures::Paths {
                binary,
                repo,
                scratch,
            },
        ),
    }
}

/// The report of a refusal: kind, hint, message, exit and the typed details.
#[must_use]
pub(crate) fn failure(error: &ReleaseError) -> Value {
    json!({
        "verdict": "fail",
        "kind": error.kind(),
        "hint": error.hint(),
        "message": error.to_string(),
        "exit": error.exit(),
        "details": error.details(),
    })
}

/// Runs `release-stage <arguments>` in-process.
#[must_use]
pub fn execute(arguments: &[String]) -> StageRun {
    let argv: Vec<String> = std::iter::once("release-stage".to_owned())
        .chain(arguments.iter().cloned())
        .collect();
    let outcome = Cli::try_parse_from(&argv)
        .map_err(|error| ReleaseError::Usage {
            detail: error
                .to_string()
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned(),
        })
        .and_then(|cli| run(cli.command));
    let (exit, mut report) = match outcome {
        Ok((exit, report)) => (exit, report),
        Err(error) => (error.exit(), failure(&error)),
    };
    if let Some(fields) = report.as_object_mut() {
        fields
            .entry("verdict")
            .or_insert_with(|| json!(if exit == 0 { "pass" } else { "fail" }));
        fields.insert("tool".to_owned(), json!("release-stage"));
        fields.insert("argv".to_owned(), json!(argv));
        fields.insert("host".to_owned(), json!(target::Platform::host()));
    }
    StageRun { exit, report, argv }
}

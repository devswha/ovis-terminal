//! The tools' shared entry point: run in-process, report as JSON, exit with the verdict.

use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;

use serde_json::{Value, json};

use super::error::QualityError;
use super::report::Verdict;
use super::{tools_audit, tools_matrix, tools_source};

/// One of the quality tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Tool {
    /// `source-check`.
    SourceCheck,
    /// `evidence-audit`.
    EvidenceAudit,
    /// `plan-audit`.
    PlanAudit,
    /// `ideal-audit`.
    IdealAudit,
    /// `quality-matrix`.
    QualityMatrix,
}

impl Tool {
    /// The binary name.
    pub const fn binary(self) -> &'static str {
        match self {
            Self::SourceCheck => "source-check",
            Self::EvidenceAudit => "evidence-audit",
            Self::PlanAudit => "plan-audit",
            Self::IdealAudit => "ideal-audit",
            Self::QualityMatrix => "quality-matrix",
        }
    }

    /// The tool a binary name or its `snake_case` spelling names.
    pub fn parse(name: &str) -> Option<Self> {
        let name = name.replace('_', "-");
        [
            Self::SourceCheck,
            Self::EvidenceAudit,
            Self::PlanAudit,
            Self::IdealAudit,
            Self::QualityMatrix,
        ]
        .into_iter()
        .find(|tool| tool.binary() == name)
    }
}

/// One finished tool run: its exit status, its JSON report and the argv it ran with.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct GateRun {
    /// 0 pass, 1 defect found, 2 usage error, 3 unreadable input.
    pub exit: u8,
    /// The report the binary prints on stdout.
    pub report: Value,
    /// Binary name followed by the arguments.
    pub argv: Vec<String>,
}

/// Where a tool's report is appended, if anywhere.
pub(super) struct Outcome {
    pub(super) verdict: Verdict,
    pub(super) output: Option<std::path::PathBuf>,
}

/// Appends `report` to `path`: a new file holds one report, an existing one becomes an
/// array of runs in invocation order, the conformance runner's `--append` convention.
fn append(path: &Path, report: &Value) -> Result<(), QualityError> {
    let document = match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(Value::Array(mut runs)) => {
                runs.push(report.clone());
                Value::Array(runs)
            }
            Ok(earlier) => Value::Array(vec![earlier, report.clone()]),
            Err(source) => {
                return Err(QualityError::Json {
                    path: path.to_path_buf(),
                    source,
                });
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => report.clone(),
        Err(error) => return Err(QualityError::io(path)(error)),
    };
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(QualityError::io(parent))?;
    }
    let bytes = serde_json::to_vec_pretty(&document).map_err(|source| QualityError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    std::fs::write(path, bytes).map_err(QualityError::io(path))
}

fn error_report(tool: Tool, error: &QualityError) -> Value {
    let mut detail = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(inner) = source {
        detail.push_str(": ");
        detail.push_str(&inner.to_string());
        source = inner.source();
    }
    let verdict = if error.exit() == 0 { "help" } else { "error" };
    json!({"gate": tool.binary(), "verdict": verdict, "exit": error.exit(), "kind": error.kind(), "hint": error.hint(), "detail": detail})
}

fn dispatch(tool: Tool, argv: &[String]) -> Result<Outcome, QualityError> {
    match tool {
        Tool::SourceCheck => tools_source::run(argv),
        Tool::EvidenceAudit => tools_audit::evidence(argv),
        Tool::PlanAudit => tools_audit::plan(argv),
        Tool::IdealAudit => tools_audit::ideal(argv),
        Tool::QualityMatrix => tools_matrix::run(argv),
    }
}

/// Runs `tool` with `arguments` in-process, exactly as its binary would.
pub fn execute(tool: Tool, arguments: &[String]) -> GateRun {
    let argv: Vec<String> = std::iter::once(tool.binary().to_owned())
        .chain(arguments.iter().cloned())
        .collect();
    let host = json!({"os": std::env::consts::OS, "arch": std::env::consts::ARCH});
    let stamp = |mut report: Value| {
        if let Some(fields) = report.as_object_mut() {
            fields.insert("argv".to_owned(), json!(argv));
            fields.insert("host".to_owned(), host.clone());
        }
        report
    };
    let (exit, report) = match dispatch(tool, &argv) {
        Ok(outcome) => {
            let report = stamp(outcome.verdict.to_json());
            match outcome
                .output
                .as_deref()
                .map(|path| append(path, &report).map(|()| path))
            {
                None => (outcome.verdict.exit(), report),
                Some(Ok(path)) => {
                    let mut report = report;
                    if let Some(fields) = report.as_object_mut() {
                        fields.insert("evidence_written".to_owned(), json!(path.to_string_lossy()));
                    }
                    (outcome.verdict.exit(), report)
                }
                Some(Err(error)) => (error.exit(), stamp(error_report(tool, &error))),
            }
        }
        Err(error) => (error.exit(), stamp(error_report(tool, &error))),
    };
    GateRun { exit, report, argv }
}

/// The binary entry point: parse argv, run, print the report, exit with its status.
pub fn main(tool: Tool) -> ExitCode {
    let args: Option<Vec<String>> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.into_string().ok())
        .collect();
    let run = args.map_or_else(
        || {
            let error = QualityError::Usage {
                detail: "arguments must be UTF-8".to_owned(),
            };
            GateRun {
                exit: error.exit(),
                report: error_report(tool, &error),
                argv: vec![tool.binary().to_owned()],
            }
        },
        |args| execute(tool, &args),
    );
    let text = serde_json::to_string_pretty(&run.report).unwrap_or_else(|_| run.report.to_string());
    if writeln!(std::io::stdout().lock(), "{text}").is_err() {
        return ExitCode::FAILURE;
    }
    ExitCode::from(run.exit)
}

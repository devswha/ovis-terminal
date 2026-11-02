//! The probe's command line, its in-process entry point and its binary main.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use serde_json::{Value, json};

use super::cases::CASES;
use super::error::InstallProbeError;

/// Runs the installer journeys of plan todo 68 against local fixture releases.
#[derive(Debug, Parser)]
#[command(name = "install-probe")]
struct Cli {
    /// The installer under test; commands run `sh install.sh` in its directory.
    #[arg(long)]
    script: PathBuf,
    /// Required: every case runs in private directories below the scratch root.
    #[arg(long)]
    isolated: bool,
    /// The binary the fixture release ships; defaults to `ovis-terminal` beside this probe.
    #[arg(long)]
    binary: Option<PathBuf>,
    /// The repository the release stage checks the binary against.
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    /// Scratch root; defaults to a fresh directory below the temporary directory.
    #[arg(long)]
    scratch: Option<PathBuf>,
    /// `all` or a comma-separated list of cases.
    #[arg(long, default_value = "all")]
    case: String,
    /// Writes the report to `<evidence stem>.install-probe.json` beside this path.
    #[arg(long)]
    evidence: Option<PathBuf>,
}

/// One finished probe run: exit status, JSON report and the argv it ran with.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct InstallRun {
    /// 0 pass, 1 a case failed, 2 usage, 3 the probe could not run.
    pub exit: u8,
    /// The report the binary prints on stdout.
    pub report: Value,
    /// `install-probe` followed by the arguments.
    pub argv: Vec<String>,
}

const fn usage(detail: String) -> InstallProbeError {
    InstallProbeError::Usage { detail }
}

fn selected(case: &str) -> Result<Vec<&'static str>, InstallProbeError> {
    if case == "all" {
        return Ok(CASES.to_vec());
    }
    case.split(',')
        .map(|name| {
            CASES
                .iter()
                .find(|known| **known == name)
                .copied()
                .ok_or_else(|| usage(format!("no case named {name}; cases: {}", CASES.join(", "))))
        })
        .collect()
}

fn scratch_root(given: Option<PathBuf>) -> Result<(PathBuf, bool), InstallProbeError> {
    let (dir, owned) = given.map_or_else(
        || {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos());
            let name = format!("ovis-install-probe-{}-{nanos}", std::process::id());
            (std::env::temp_dir().join(name), true)
        },
        |dir| (dir, false),
    );
    std::fs::create_dir_all(&dir).map_err(InstallProbeError::io("create", &dir))?;
    Ok((dir, owned))
}

fn checkout_of(script: &Path) -> Result<PathBuf, InstallProbeError> {
    let script = script
        .canonicalize()
        .map_err(InstallProbeError::io("resolve", script))?;
    if script.file_name().and_then(|name| name.to_str()) != Some("install.sh") {
        return Err(usage(format!("{} is not an install.sh", script.display())));
    }
    script
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| usage("install.sh has no directory".to_owned()))
}

fn probe(cli: &Cli) -> Result<(u8, Value), InstallProbeError> {
    if !cli.isolated {
        return Err(usage("the probe runs only with --isolated".to_owned()));
    }
    let cases = selected(&cli.case)?;
    let checkout = checkout_of(&cli.script)?;
    let binary = match &cli.binary {
        Some(binary) => binary.clone(),
        None => std::env::current_exe()
            .map_err(InstallProbeError::io("locate", Path::new("install-probe")))?
            .with_file_name(format!("ovis-terminal{}", std::env::consts::EXE_SUFFIX)),
    };
    let (scratch, owned) = scratch_root(cli.scratch.clone())?;
    let outcome = super::report::cases(&super::report::Setup {
        cases: &cases,
        checkout: &checkout,
        binary: &binary,
        repo: &cli.repo,
        scratch: &scratch,
    });
    if owned {
        std::fs::remove_dir_all(&scratch).map_err(InstallProbeError::io("remove", &scratch))?;
    }
    outcome
}

/// Runs `install-probe <arguments>` in-process.
#[must_use]
pub fn execute(arguments: &[String]) -> InstallRun {
    let argv: Vec<String> = std::iter::once("install-probe".to_owned())
        .chain(arguments.iter().cloned())
        .collect();
    let parsed = Cli::try_parse_from(&argv).map_err(|error| {
        usage(
            error
                .to_string()
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned(),
        )
    });
    let evidence = parsed.as_ref().ok().and_then(|cli| cli.evidence.clone());
    let (exit, mut report) = match parsed.and_then(|cli| probe(&cli)) {
        Ok(done) => done,
        Err(error) => (
            error.exit(),
            json!({ "verdict": "fail", "kind": error.kind(), "hint": error.hint(), "message": error.to_string() }),
        ),
    };
    if let Some(fields) = report.as_object_mut() {
        fields.insert("tool".to_owned(), json!("install-probe"));
        fields.insert("argv".to_owned(), json!(argv));
        fields.insert("exit".to_owned(), json!(exit));
    }
    if let Some(path) = evidence {
        let companion = path.with_extension("install-probe.json");
        let text = serde_json::to_vec_pretty(&report).unwrap_or_default();
        if let Err(source) = std::fs::write(&companion, text) {
            let error = InstallProbeError::io("write", &companion)(source);
            return InstallRun {
                exit: error.exit(),
                report: json!({ "verdict": "fail", "kind": error.kind(), "hint": error.hint(), "message": error.to_string() }),
                argv,
            };
        }
    }
    InstallRun { exit, report, argv }
}

/// The `install-probe` binary: prints the report and exits with its status.
#[must_use]
pub fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let run = execute(&arguments);
    let text = serde_json::to_string_pretty(&run.report).unwrap_or_default();
    if writeln!(std::io::stdout().lock(), "{text}").is_err() {
        return ExitCode::from(3);
    }
    ExitCode::from(run.exit)
}

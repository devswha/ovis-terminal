//! `workspace-hygiene check [--root <dir>] [--fixture <name>]`.
//!
//! Exit status: 0 when every file passes, 1 when any rule is violated, 2 for a
//! usage error and 3 when files cannot be read.

use std::ffi::OsString;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use workspace_hygiene::check::check_source;
use workspace_hygiene::fixtures;
use workspace_hygiene::report::Violation;
use workspace_hygiene::walk::{self, WalkError};

const USAGE: &str = "usage: workspace-hygiene check [--root <dir>] [--fixture <name>]";

/// What to check.
enum Target {
    Workspace(PathBuf),
    Fixture(String),
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let Some(target) = parse(&args) else {
        return report_line(USAGE, 2);
    };
    let result = match target {
        Target::Workspace(root) => check_workspace(&root),
        Target::Fixture(name) => match fixtures::find(&name) {
            Some(fixture) => Ok((check_source(Path::new(fixture.path), fixture.source), 1)),
            None => return report_line(&format!("unknown fixture `{name}`\n{USAGE}"), 2),
        },
    };
    match result {
        Ok((violations, files)) => print_report(&violations, files),
        Err(error) => report_line(&format!("workspace-hygiene: {error}"), 3),
    }
}

fn parse(args: &[OsString]) -> Option<Target> {
    match args {
        [command] if command == "check" => Some(Target::Workspace(PathBuf::from("."))),
        [command, flag, value] if command == "check" && flag == "--root" => {
            Some(Target::Workspace(PathBuf::from(value)))
        }
        [command, flag, value] if command == "check" && flag == "--fixture" => {
            value.to_str().map(|name| Target::Fixture(name.to_owned()))
        }
        _ => None,
    }
}

fn check_workspace(root: &Path) -> Result<(Vec<Violation>, usize), WalkError> {
    let sources = walk::rust_sources(root)?;
    let mut violations = Vec::new();
    for path in &sources {
        let source = walk::read_source(path)?;
        let shown = path.strip_prefix(root).unwrap_or(path);
        violations.extend(check_source(shown, &source));
    }
    Ok((violations, sources.len()))
}

fn print_report(violations: &[Violation], files: usize) -> ExitCode {
    let mut out = io::stdout().lock();
    let written = violations
        .iter()
        .try_for_each(|violation| writeln!(out, "{violation}"))
        .and_then(|()| {
            writeln!(
                out,
                "workspace-hygiene: {} violation(s) in {files} file(s) checked",
                violations.len()
            )
        });
    match (written, violations.is_empty()) {
        (Err(_), _) => ExitCode::FAILURE,
        (Ok(()), true) => ExitCode::SUCCESS,
        (Ok(()), false) => ExitCode::from(1),
    }
}

fn report_line(message: &str, status: u8) -> ExitCode {
    match writeln!(io::stderr().lock(), "{message}") {
        Ok(()) => ExitCode::from(status),
        Err(_) => ExitCode::FAILURE,
    }
}

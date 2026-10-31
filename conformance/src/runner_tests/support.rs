//! Running the real runner binary and building purpose-built scenario trees.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;

/// Result of a test step; any failure fails the test.
pub type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// The binary under test: `$OVIS_TERMINAL_BINARY`, or `ovis-terminal` beside the runner.
///
/// # Errors
///
/// Fails when that file does not exist; the test is never skipped.
pub fn binary() -> TestResult<PathBuf> {
    let runner = Path::new(env!("CARGO_BIN_EXE_conformance"));
    let path = std::env::var_os("OVIS_TERMINAL_BINARY")
        .map_or_else(|| runner.with_file_name("ovis-terminal"), PathBuf::from);
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "{} is missing: cargo build -p ovis-terminal-cli",
            path.display()
        )
        .into())
    }
}

/// One finished runner process and the evidence file it wrote.
#[derive(Debug)]
#[non_exhaustive]
pub struct Ran {
    /// Exit status and both captured streams.
    pub output: Output,
    /// The parsed evidence file, or null when none was written.
    pub evidence: Value,
}

impl Ran {
    /// The summary line plus every failed case's typed failure and failed assertions.
    pub fn explain(&self) -> String {
        let mut lines = vec![String::from_utf8_lossy(&self.output.stdout).into_owned()];
        let cases = self.evidence.pointer("/cases").and_then(Value::as_array);
        for case in cases.into_iter().flatten() {
            if field(case, "/outcome") == "passed" {
                continue;
            }
            let (scenario, name) = (field(case, "/scenario"), field(case, "/case"));
            lines.push(format!("{scenario} {name}: {}", field(case, "/failure")));
            let assertions = field(case, "/assertions").as_array().into_iter().flatten();
            lines.extend(
                assertions
                    .filter(|assertion| field(assertion, "/passed") != true)
                    .map(|failed| format!("  {failed}")),
            );
        }
        lines.join("\n")
    }
}

/// The value at a JSON pointer, or null when absent.
pub fn field<'a>(value: &'a Value, pointer: &str) -> &'a Value {
    value.pointer(pointer).unwrap_or(&Value::Null)
}

/// Runs the runner on `scenario`/`case` against the binary under test, with `extra` flags.
///
/// # Errors
///
/// Fails when the binary is missing or the runner cannot be spawned.
pub fn conformance(scenario: &str, case: &str, extra: &[&str]) -> TestResult<Ran> {
    // Unique per call: two tests may run the same scenario and case concurrently.
    let dir = std::env::temp_dir().join(format!(
        "ovc-test-{}-{}-{case}-{}",
        std::process::id(),
        scenario.replace('/', "-"),
        uuid::Uuid::now_v7()
    ));
    std::fs::create_dir_all(&dir)?;
    let evidence = dir.join(format!("{case}.json"));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_conformance"))
        .arg("--binary")
        .arg(binary()?)
        .args([
            "--scenario",
            scenario,
            "--isolated",
            "--case",
            case,
            "--evidence",
        ])
        .arg(&evidence)
        .args(extra)
        .output()?;
    let evidence = std::fs::read(&evidence).map_or(Value::Null, |bytes| {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    });
    std::fs::remove_dir_all(&dir)?;
    Ok(Ran { output, evidence })
}

/// The first evidence assertion with this check name, in any case.
pub fn assertion<'a>(evidence: &'a Value, check: &str) -> Option<&'a Value> {
    evidence
        .pointer("/cases")?
        .as_array()?
        .iter()
        .filter_map(|case| case.get("assertions")?.as_array())
        .flatten()
        .find(|assertion| assertion.get("check").and_then(Value::as_str) == Some(check))
}

/// The sorted strings, or `file` fields, of the array at `pointer`.
pub fn strings(value: &Value, pointer: &str) -> Vec<String> {
    let items = value
        .pointer(pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut names: Vec<String> = items
        .iter()
        .filter_map(|item| {
            item.as_str()
                .or_else(|| item.get("file").and_then(Value::as_str))
        })
        .map(str::to_owned)
        .collect();
    names.sort();
    names
}

/// A scenario tree in a fresh directory, removed on drop.
#[derive(Debug)]
pub struct Tree(PathBuf);

impl Tree {
    /// Writes `files` (relative path, content) below a fresh directory.
    ///
    /// # Errors
    ///
    /// Fails when a directory or file cannot be written.
    pub fn new(name: &str, files: &[(&str, String)]) -> TestResult<Self> {
        let root = std::env::temp_dir().join(format!("ovc-tree-{}-{name}", std::process::id()));
        for (relative, text) in files {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().ok_or("parent")?)?;
            std::fs::write(path, text)?;
        }
        Ok(Self(root))
    }

    /// The directory as a `--scenario-root` argument.
    pub fn arg(&self) -> String {
        self.0.display().to_string()
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            assert!(std::thread::panicking(), "cleanup: {error}");
        }
    }
}

/// A registered scenario whose one case runs `--version` and expects `exit`.
pub fn scenario(id: &str, exit: i32) -> String {
    serde_json::json!({
        "format": "ovis-conformance/1", "scenario": id, "title": "purpose-built", "todo": 21,
        "platforms": ["linux", "macos"],
        "cases": [{"case": "happy", "summary": "runs --version", "steps": [
            {"run": {"args": ["--version"], "exit": exit}}
        ]}],
    })
    .to_string()
}

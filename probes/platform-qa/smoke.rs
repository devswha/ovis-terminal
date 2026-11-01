//! The Windows smoke manifest, the report a run writes and its verification.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::error::QaError;

/// Format marker of the smoke manifest.
pub const MANIFEST_FORMAT: &str = "ovis-windows-smoke-manifest/1";
/// Format marker of the smoke report.
pub const REPORT_FORMAT: &str = "ovis-windows-smoke/1";

/// Named checks, each proven by native tests run one at a time.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SmokeManifest {
    /// Always [`MANIFEST_FORMAT`].
    pub format: String,
    /// The checks, in run order.
    pub checks: Vec<Check>,
}

/// One named contract check.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Check {
    /// Stable check name, such as `named-pipe-hello`.
    pub name: String,
    /// What a pass observes on the runner.
    pub observes: String,
    /// Native tests that must each run exactly once and pass.
    pub tests: Vec<NativeTest>,
}

/// One libtest test addressed by package, target and exact path.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct NativeTest {
    /// Cargo package.
    pub package: String,
    /// `lib`, or the name of an integration test target.
    pub target: String,
    /// Exact test path inside that target.
    pub test: String,
    /// The test is `#[ignore]`d outside this job and must be included.
    pub ignored: bool,
}

impl NativeTest {
    /// The `cargo test` arguments that run exactly this test.
    pub fn cargo_args(&self) -> Vec<String> {
        let mut args: Vec<String> = ["test", "--locked", "-p", &self.package]
            .map(str::to_owned)
            .to_vec();
        if self.target == "lib" {
            args.push("--lib".to_owned());
        } else {
            args.extend(["--test".to_owned(), self.target.clone()]);
        }
        args.extend([self.test.clone(), "--".to_owned(), "--exact".to_owned()]);
        if self.ignored {
            args.push("--include-ignored".to_owned());
        }
        args
    }
}

impl SmokeManifest {
    /// Reads and checks a manifest: known format, unique non-empty check names, tests.
    ///
    /// # Errors
    ///
    /// The file is unreadable, not the manifest shape, or breaks one of those rules.
    pub fn load(path: &Path) -> Result<Self, QaError> {
        let bytes = std::fs::read(path).map_err(|source| QaError::Io {
            operation: "read the smoke manifest",
            source,
        })?;
        let manifest: Self = serde_json::from_slice(&bytes).map_err(|source| QaError::Json {
            what: path.display().to_string(),
            source,
        })?;
        let invalid = |detail: &str| QaError::ManifestInvalid {
            file: path.to_path_buf(),
            detail: detail.to_owned(),
        };
        if manifest.format != MANIFEST_FORMAT {
            return Err(invalid("unknown format marker"));
        }
        let mut names: Vec<&str> = manifest.checks.iter().map(|c| c.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        if names.len() != manifest.checks.len() || names.is_empty() {
            return Err(invalid("check names must be present and unique"));
        }
        if manifest.checks.iter().any(|check| check.tests.is_empty()) {
            return Err(invalid("every check names at least one native test"));
        }
        Ok(manifest)
    }
}

/// Pass or fail.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Verdict {
    /// Every part passed.
    Pass,
    /// At least one part failed.
    Fail,
}

/// What one smoke run proved, written next to the captured logs.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SmokeReport {
    /// Always [`REPORT_FORMAT`].
    pub format: String,
    /// The commit that was built and tested.
    pub sha: String,
    /// Native operating system of the runner.
    pub os: String,
    /// Native architecture of the runner.
    pub arch: String,
    /// What this tier is and is not.
    pub scope: String,
    /// Pass only when every check passed.
    pub windows_smoke: Verdict,
    /// Every check, in run order.
    pub checks: Vec<CheckReport>,
}

/// The outcome of one check.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CheckReport {
    /// The check name.
    pub name: String,
    /// Pass only when every test passed.
    pub status: Verdict,
    /// What a pass observes.
    pub observes: String,
    /// Every test run.
    pub tests: Vec<TestRun>,
}

/// One captured native test run.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TestRun {
    /// The full command line.
    pub argv: Vec<String>,
    /// Exit code, absent when the deadline killed it.
    pub exit_code: Option<i32>,
    /// The deadline ended the run.
    pub timed_out: bool,
    /// Whether the output proves the test ran once and passed.
    pub verdict: Verdict,
    /// Why it is not proven, when it is not.
    pub failure: Option<String>,
    /// Log file, relative to the report.
    pub log: String,
    /// SHA-256 of the captured standard output.
    pub stdout_sha256: String,
    /// SHA-256 of the captured standard error.
    pub stderr_sha256: String,
}

impl SmokeReport {
    /// Checks the report is for `sha`, consistent, and passes every `required` check.
    ///
    /// # Errors
    ///
    /// Another commit, a verdict contradicting its checks, or a required check that is
    /// missing or did not pass.
    pub fn verify(&self, sha: &str, required: &[String]) -> Result<(), QaError> {
        let inconsistent = |detail: &str| QaError::ReportInconsistent {
            detail: detail.to_owned(),
        };
        if self.format != REPORT_FORMAT {
            return Err(inconsistent("unknown report format"));
        }
        if self.sha != sha {
            return Err(QaError::ShaMismatch {
                expected: sha.to_owned(),
                reported: self.sha.clone(),
            });
        }
        for check in &self.checks {
            let all = check.tests.iter().all(|run| run.verdict == Verdict::Pass);
            if (check.status == Verdict::Pass) != (all && !check.tests.is_empty()) {
                return Err(inconsistent("a check status contradicts its tests"));
            }
        }
        let every = !self.checks.is_empty()
            && self
                .checks
                .iter()
                .all(|check| check.status == Verdict::Pass);
        if (self.windows_smoke == Verdict::Pass) != every {
            return Err(inconsistent("windows_smoke contradicts the checks"));
        }
        for name in required {
            let check = self
                .checks
                .iter()
                .find(|check| &check.name == name)
                .ok_or_else(|| QaError::CheckMissing { name: name.clone() })?;
            if check.status != Verdict::Pass {
                return Err(QaError::CheckFailed { name: name.clone() });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "smoke_tests.rs"]
mod tests;

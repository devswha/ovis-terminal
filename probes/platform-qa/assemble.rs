//! Building the smoke report from captured runs, so its verdicts follow one rule.

use sha2::{Digest as _, Sha256};

use super::bounded::Bounded;
use super::libtest::proven;
use super::smoke::{Check, CheckReport, NativeTest, REPORT_FORMAT, SmokeReport, TestRun, Verdict};

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            hex.extend([nibble(byte >> 4), nibble(byte & 0x0f)]);
            hex
        })
}

fn nibble(value: u8) -> char {
    char::from_digit(u32::from(value), 16).unwrap_or('0')
}

const fn verdict(pass: bool) -> Verdict {
    if pass { Verdict::Pass } else { Verdict::Fail }
}

impl TestRun {
    /// The record of `test` run as `done`, whose log was written to `log`.
    pub fn captured(test: &NativeTest, done: &Bounded, log: &str) -> Self {
        let proof = proven(
            &String::from_utf8_lossy(&done.stdout),
            &test.test,
            done.exit_code,
        );
        let mut argv = vec!["cargo".to_owned()];
        argv.extend(test.cargo_args());
        Self {
            argv,
            exit_code: done.exit_code,
            timed_out: done.timed_out,
            verdict: verdict(proof.is_ok()),
            failure: proof.err().map(|reason| format!("{reason:?}")),
            log: log.to_owned(),
            stdout_sha256: sha256_hex(&done.stdout),
            stderr_sha256: sha256_hex(&done.stderr),
        }
    }
}

impl CheckReport {
    /// `check` with its runs; it passes only when there is a run and every run passed.
    pub fn of(check: &Check, tests: Vec<TestRun>) -> Self {
        let pass = !tests.is_empty() && tests.iter().all(|run| run.verdict == Verdict::Pass);
        Self {
            name: check.name.clone(),
            status: verdict(pass),
            observes: check.observes.clone(),
            tests,
        }
    }
}

impl SmokeReport {
    /// The report of this runner at `sha`; it passes only when every check passed.
    pub fn assemble(sha: &str, checks: Vec<CheckReport>) -> Self {
        let every = checks.iter().all(|check| check.status == Verdict::Pass);
        Self {
            format: REPORT_FORMAT.to_owned(),
            sha: sha.to_owned(),
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            scope: "Windows contract smoke tier on a hosted runner; not full Windows manual QA."
                .to_owned(),
            windows_smoke: verdict(every && !checks.is_empty()),
            checks,
        }
    }

    /// How many checks failed.
    pub fn failed(&self) -> usize {
        self.checks
            .iter()
            .filter(|check| check.status == Verdict::Fail)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::{CheckReport, SmokeReport, TestRun, sha256_hex};
    use crate::platform_qa::{Bounded, Check, NativeTest, Verdict};

    fn test() -> NativeTest {
        NativeTest {
            package: "ovis-terminal-holder".to_owned(),
            target: "lib".to_owned(),
            test: "windows::terminal_tests::kill".to_owned(),
            ignored: false,
        }
    }

    fn done(stdout: &str, exit_code: Option<i32>) -> Bounded {
        Bounded {
            exit_code,
            timed_out: exit_code.is_none(),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    const PASSED: &str = "test windows::terminal_tests::kill ... ok\n\
        test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out\n";

    #[test]
    fn a_proven_run_passes_and_a_timed_out_run_fails_its_check() {
        let ok = TestRun::captured(&test(), &done(PASSED, Some(0)), "kill-0.log");
        assert_eq!(ok.verdict, Verdict::Pass);
        assert_eq!(ok.stdout_sha256, sha256_hex(PASSED.as_bytes()));
        assert_eq!(ok.argv.first().map(String::as_str), Some("cargo"));
        let hung = TestRun::captured(&test(), &done("", None), "kill-1.log");
        assert_eq!(hung.verdict, Verdict::Fail);
        assert!(hung.timed_out);
        let check = Check {
            name: "conpty-kill".to_owned(),
            observes: String::new(),
            tests: vec![test(), test()],
        };
        let report = SmokeReport::assemble("abc", vec![CheckReport::of(&check, vec![ok, hung])]);
        assert_eq!(report.windows_smoke, Verdict::Fail);
        assert_eq!(report.failed(), 1);
    }

    #[test]
    fn a_report_with_no_check_does_not_pass() {
        assert_eq!(
            SmokeReport::assemble("abc", Vec::new()).windows_smoke,
            Verdict::Fail
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}

use super::{CheckReport, NativeTest, REPORT_FORMAT, SmokeManifest, SmokeReport, TestRun, Verdict};
use crate::platform_qa::{QaError, sha256_hex};

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

fn run(verdict: Verdict) -> TestRun {
    TestRun {
        argv: vec!["cargo".to_owned(), "test".to_owned()],
        exit_code: Some(i32::from(verdict == Verdict::Fail)),
        timed_out: false,
        verdict,
        failure: None,
        log: "named-pipe-hello-0.log".to_owned(),
        stdout_sha256: sha256_hex(b"out"),
        stderr_sha256: sha256_hex(b"err"),
    }
}

fn check(name: &str, verdict: Verdict) -> CheckReport {
    CheckReport {
        name: name.to_owned(),
        status: verdict,
        observes: String::new(),
        tests: vec![run(verdict)],
    }
}

fn report(checks: Vec<CheckReport>) -> SmokeReport {
    let every = checks.iter().all(|check| check.status == Verdict::Pass);
    SmokeReport {
        format: REPORT_FORMAT.to_owned(),
        sha: SHA.to_owned(),
        os: "windows".to_owned(),
        arch: "x86_64".to_owned(),
        scope: String::new(),
        windows_smoke: if every { Verdict::Pass } else { Verdict::Fail },
        checks,
    }
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn every_required_check_passing_at_the_commit_verifies() {
    let report = report(vec![
        check("named-pipe-hello", Verdict::Pass),
        check("conpty-kill", Verdict::Pass),
    ]);
    report
        .verify(SHA, &names(&["named-pipe-hello", "conpty-kill"]))
        .unwrap();
}

#[test]
fn a_missing_or_failed_required_check_is_named() {
    let report = report(vec![
        check("named-pipe-hello", Verdict::Pass),
        check("conpty-kill", Verdict::Fail),
    ]);
    let missing = report.verify(SHA, &names(&["two-clients"])).unwrap_err();
    assert!(matches!(&missing, QaError::CheckMissing { name } if name == "two-clients"));
    let failed = report.verify(SHA, &names(&["conpty-kill"])).unwrap_err();
    assert!(matches!(&failed, QaError::CheckFailed { name } if name == "conpty-kill"));
    assert_eq!(failed.kind(), "check_failed");
}

#[test]
fn a_report_for_another_commit_is_refused() {
    let report = report(vec![check("named-pipe-hello", Verdict::Pass)]);
    let refused = report.verify("feedface", &[]).unwrap_err();
    assert!(matches!(refused, QaError::ShaMismatch { .. }));
}

#[test]
fn a_pass_verdict_over_a_failed_test_is_inconsistent() {
    let mut forged = report(vec![check("conpty-kill", Verdict::Pass)]);
    if let Some(first) = forged.checks.first_mut() {
        first.tests = vec![run(Verdict::Fail)];
    }
    let refused = forged.verify(SHA, &names(&["conpty-kill"])).unwrap_err();
    assert_eq!(refused.kind(), "report_inconsistent");

    let mut overall = report(vec![check("conpty-kill", Verdict::Fail)]);
    overall.windows_smoke = Verdict::Pass;
    assert_eq!(
        overall.verify(SHA, &[]).unwrap_err().kind(),
        "report_inconsistent"
    );
}

#[test]
fn a_check_with_no_test_cannot_pass() {
    let mut empty = report(vec![check("two-clients", Verdict::Pass)]);
    if let Some(first) = empty.checks.first_mut() {
        first.tests.clear();
    }
    assert_eq!(
        empty.verify(SHA, &[]).unwrap_err().kind(),
        "report_inconsistent"
    );
}

#[test]
fn an_ignored_lib_test_is_run_exactly_and_included() {
    let test = NativeTest {
        package: "ovis-terminal-cli".to_owned(),
        target: "lib".to_owned(),
        test: "service::windows::fixture_tests::outlives".to_owned(),
        ignored: true,
    };
    assert_eq!(
        test.cargo_args(),
        names(&[
            "test",
            "--locked",
            "-p",
            "ovis-terminal-cli",
            "--lib",
            "service::windows::fixture_tests::outlives",
            "--",
            "--exact",
            "--include-ignored",
        ])
    );
    let integration = NativeTest {
        target: "pipe_smoke".to_owned(),
        ignored: false,
        ..test
    };
    assert_eq!(
        integration.cargo_args().get(4..6),
        Some(names(&["--test", "pipe_smoke"]).as_slice())
    );
    assert_eq!(
        integration.cargo_args().last().map(String::as_str),
        Some("--exact")
    );
}

#[test]
fn an_empty_commit_or_a_run_without_output_hashes_is_refused() {
    let report = report(vec![check("conpty-kill", Verdict::Pass)]);
    let empty = report.verify("", &[]).unwrap_err();
    assert_eq!(empty.kind(), "usage");

    let mut unhashed = report;
    if let Some(run) = unhashed
        .checks
        .first_mut()
        .and_then(|c| c.tests.first_mut())
    {
        run.stderr_sha256 = String::new();
    }
    assert_eq!(
        unhashed.verify(SHA, &[]).unwrap_err().kind(),
        "report_inconsistent"
    );
}

#[test]
fn a_manifest_check_with_no_native_test_is_refused_at_load() {
    let dir = std::env::temp_dir().join(format!("ovis-qa-manifest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("windows-smoke.json");
    let check = |tests: &str| {
        format!(
            r#"{{"format":"ovis-windows-smoke-manifest/1","checks":[{{"name":"two-clients","observes":"x","tests":{tests}}}]}}"#
        )
    };
    std::fs::write(&path, check("[]")).unwrap();
    let refused = SmokeManifest::load(&path).unwrap_err();
    assert_eq!(refused.kind(), "manifest_invalid");
    let one = r#"[{"package":"p","target":"lib","test":"t","ignored":false}]"#;
    std::fs::write(&path, check(one)).unwrap();
    assert_eq!(SmokeManifest::load(&path).unwrap().checks.len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

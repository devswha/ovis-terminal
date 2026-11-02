//! The promotion gate against the synthetic five-target matrix, through [`crate::execute`].

use std::path::PathBuf;

use serde_json::Value;

fn run(fixture: &str) -> (u8, Value) {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let scratch = std::env::var_os("OVIS_FIXTURE_ROOT")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join(format!("release-stage-tests-{}", std::process::id()));
    let args: Vec<String> = [
        "fixture",
        fixture,
        "--binary",
        "unused-by-matrix-fixtures",
        "--repo",
        &repo.to_string_lossy(),
        "--scratch",
        &scratch.to_string_lossy(),
    ]
    .map(str::to_owned)
    .into();
    let run = crate::execute(&args);
    let removed = std::fs::remove_dir_all(scratch.join(format!("release-{fixture}")));
    assert!(removed.is_ok(), "{removed:?}");
    (run.exit, run.report)
}

fn at(report: &Value, pointer: &str) -> Value {
    report.pointer(pointer).cloned().unwrap_or(Value::Null)
}

fn kind(report: &Value) -> Option<&str> {
    report.get("kind").and_then(Value::as_str)
}

#[test]
fn a_complete_native_matrix_promotes_only_as_a_dry_run_that_publishes_nothing() {
    let (exit, report) = run("complete-matrix-dry-run");
    assert_eq!(exit, 0, "{report}");
    assert_eq!(at(&report, "/outcome"), "dry_run");
    assert_eq!(at(&report, "/publish"), false);
    assert_eq!(at(&report, "/targets"), "5");
    assert_eq!(at(&report, "/signing/performed"), false);
}

#[test]
fn each_missing_or_foreign_part_refuses_promotion_with_its_kind() {
    for (fixture, expected) in [
        ("missing-windows-artifact", "target_missing"),
        ("missing-native-qa", "native_qa_missing"),
        ("cross-built-macos", "cross_build_refused"),
        ("unpinned-sqlite", "target_receipt_invalid"),
        ("bad-manifest-hash", "checksum_mismatch"),
        ("malformed-manifest", "manifest_malformed"),
        ("conflicting-rerun", "release_conflict"),
    ] {
        let (exit, report) = run(fixture);
        assert_eq!(
            (exit, kind(&report)),
            (1, Some(expected)),
            "{fixture}: {report}"
        );
        assert_ne!(report.get("publish"), Some(&Value::Bool(true)), "{fixture}");
    }
}

#[test]
fn a_rerun_after_publication_never_publishes_again() {
    let (exit, report) = run("rerun-after-publish");
    assert_eq!(exit, 0, "{report}");
    assert_eq!(at(&report, "/runs/0/outcome"), "promote");
    assert_eq!(at(&report, "/runs/0/publish"), true);
    assert_eq!(at(&report, "/runs/1/outcome"), "already_published");
    assert_eq!(at(&report, "/runs/1/publish"), false);
}

#[test]
fn an_unknown_fixture_or_flag_is_a_usage_error() {
    assert_eq!(run("no-such-fixture").0, 2);
    let run = crate::execute(&["promote".to_owned(), "--bogus".to_owned()]);
    assert_eq!((run.exit, kind(&run.report)), (2, Some("usage")));
}

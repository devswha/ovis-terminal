//! The released baseline changes only with an entry in its amendment ledger.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ovis_terminal_protocol::schema::{SchemaError, baseline_digest, check_amendments};
use serde_json::{Value, json};

#[cfg(test)]
fn schema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema")
}

#[cfg(test)]
fn released() -> Value {
    let text = std::fs::read_to_string(schema_dir().join("released.openrpc.json"))
        .expect("baseline is readable");
    serde_json::from_str(&text).expect("baseline is JSON")
}

#[cfg(test)]
fn ledger() -> String {
    std::fs::read_to_string(schema_dir().join("BASELINE-AMENDMENTS.md"))
        .expect("ledger is readable")
}

/// The baseline with a compatible edit: a description nobody decodes.
#[cfg(test)]
fn amended() -> Value {
    let mut baseline = released();
    *baseline
        .pointer_mut("/components/schemas/Count/description")
        .expect("Count has a description") = json!("Count, amended without an entry.");
    baseline
}

#[cfg(test)]
fn entry(previous: &str, current: &str) -> String {
    format!(
        "\n## test amendment\n\n- date: 2026-11-01\n- pr: #0\n- fields: Count.description\n- reason: test\n- previous: sha256:{previous}\n- current: sha256:{current}\n"
    )
}

#[test]
fn shipped_ledger_records_the_shipped_baseline() {
    check_amendments(&released(), &ledger()).expect("every amendment is recorded");
}

#[test]
fn digest_follows_content_not_formatting() {
    let baseline = released();
    let reformatted: Value =
        serde_json::from_str(&serde_json::to_string(&baseline).expect("encodes")).expect("JSON");
    assert_eq!(
        baseline_digest(&baseline).expect("digest"),
        baseline_digest(&reformatted).expect("digest")
    );
    assert_ne!(
        baseline_digest(&baseline).expect("digest"),
        baseline_digest(&amended()).expect("digest")
    );
}

#[test]
fn unrecorded_change_names_the_missing_entry() {
    let head = baseline_digest(&released()).expect("digest");
    let edited = amended();
    let digest = baseline_digest(&edited).expect("digest");
    let error = check_amendments(&edited, &ledger()).expect_err("no entry records the edit");
    assert_eq!(error.kind(), "baseline_amendment_missing");
    let SchemaError::AmendmentMissing {
        digest: found,
        previous,
    } = &error
    else {
        panic!("expected a missing amendment, got {error:?}")
    };
    assert_eq!((found, previous), (&digest, &head));
    let recorded = format!("{}{}", ledger(), entry(&head, &digest));
    check_amendments(&edited, &recorded).expect("the new entry records the edit");
}

#[test]
fn incomplete_or_unchained_entries_are_malformed() {
    let head = baseline_digest(&released()).expect("digest");
    let digest = baseline_digest(&amended()).expect("digest");
    let location = |ledger: &str| match check_amendments(&amended(), ledger) {
        Err(SchemaError::AmendmentsMalformed { location }) => location,
        other => panic!("expected a malformed ledger, got {other:?}"),
    };
    let unreasoned =
        format!("{}{}", ledger(), entry(&head, &digest)).replace("- reason: test\n", "");
    assert_eq!(location(&unreasoned), "test amendment.reason");
    let unchained = format!("{}{}", ledger(), entry(&digest, &digest));
    assert_eq!(location(&unchained), "test amendment.previous");
    let unreleased = ledger().replace("- released:", "- first:");
    assert_eq!(location(&unreleased), "released");
}

#[cfg(test)]
fn compat(dir: &Path) -> (Output, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_openrpc"))
        .arg("compat")
        .arg(dir.join("released.openrpc.json"))
        .output()
        .expect("runs");
    let stream = if output.status.success() {
        &output.stdout
    } else {
        &output.stderr
    };
    let report = serde_json::from_slice(stream).unwrap_or_else(|_| {
        panic!(
            "compat printed no JSON: status {:?} stdout {} stderr {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output, report)
}

#[test]
fn cli_compat_requires_a_ledger_entry_for_a_changed_baseline() {
    let dir = std::env::temp_dir().join(format!("ovis-amendments-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).expect("writes");
    let pretty = serde_json::to_string_pretty(&amended()).expect("encodes");
    write("released.openrpc.json", &pretty);
    let (missing_ledger, report) = compat(&dir);
    assert_eq!(
        (missing_ledger.status.code(), report.get("kind")),
        (Some(1), Some(&json!("io")))
    );
    write("BASELINE-AMENDMENTS.md", &ledger());
    let (refused, report) = compat(&dir);
    let digest = baseline_digest(&amended()).expect("digest");
    assert_eq!(
        (refused.status.code(), report.get("kind")),
        (Some(1), Some(&json!("baseline_amendment_missing")))
    );
    let message = report
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(message.contains(&digest), "{report}");
    let head = baseline_digest(&released()).expect("digest");
    write(
        "BASELINE-AMENDMENTS.md",
        &format!("{}{}", ledger(), entry(&head, &digest)),
    );
    let (accepted, report) = compat(&dir);
    std::fs::remove_dir_all(&dir).expect("cleanup");
    assert_eq!(accepted.status.code(), Some(0), "{report}");
}

//! The repository passes its own gates, and the gates notice drift in what they guard.

pub mod support;

use std::path::{Path, PathBuf};

use ovis_terminal_probes::quality::Tool;
use serde_json::Value;

use self::support::{field, run};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn repo_path(relative: &str) -> String {
    repo().join(relative).to_string_lossy().into_owned()
}

#[test]
fn the_repository_passes_source_check_with_a_fresh_checklist() {
    let root = repo_path("");
    let ran = run(
        Tool::SourceCheck,
        &[
            "--repo",
            &root,
            "--roots",
            "crates",
            "packages/client-ts",
            "conformance",
            "bench",
            "probes",
        ],
    );
    assert_eq!(ran.exit, 0, "{}", ran.report);
    assert_eq!(field(&ran.report, "/checklist/rule_ids"), "64");
    assert_eq!(field(&ran.report, "/checklist/v_rows"), "5174");
}

#[test]
fn the_matrices_cover_every_ledger_row_and_the_schema_still_holds_their_contract() {
    let ran = run(
        Tool::QualityMatrix,
        &[
            "--mode",
            "verify",
            "--ideals",
            &repo_path("docs/design/ideal-state.md"),
            "--gaps",
            &repo_path("docs/design/gap-list.md"),
            "--schema",
            &repo_path("schema/ovis-terminal.openrpc.json"),
            "--traceability",
            &repo_path("conformance/traceability.json"),
            "--ideal-map",
            &repo_path("conformance/ideal-map.json"),
        ],
    );
    assert_eq!(ran.exit, 0, "{}", ran.report);
    assert_eq!(field(&ran.report, "/ideals"), "121");
}

#[test]
fn a_drifted_predicate_or_a_removed_method_fails_the_matrix_check()
-> Result<(), Box<dyn std::error::Error>> {
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("quality-matrix-drift");
    std::fs::create_dir_all(&scratch)?;
    let mut map: Value =
        serde_json::from_slice(&std::fs::read(repo().join("conformance/ideal-map.json"))?)?;
    if let Some(predicate) = map.pointer_mut("/ideals/IDEAL-008/predicate") {
        *predicate = Value::from("A weaker statement.");
    }
    if let Some(Value::Array(methods)) = map.pointer_mut("/contract/methods") {
        methods.push(Value::from("removed.method"));
    }
    let drifted = scratch.join("ideal-map.json");
    std::fs::write(&drifted, serde_json::to_vec(&map)?)?;
    let drifted = drifted.to_string_lossy().into_owned();
    let ran = run(
        Tool::QualityMatrix,
        &[
            "--mode",
            "verify",
            "--ideals",
            &repo_path("docs/design/ideal-state.md"),
            "--gaps",
            &repo_path("docs/design/gap-list.md"),
            "--schema",
            &repo_path("schema/ovis-terminal.openrpc.json"),
            "--traceability",
            &repo_path("conformance/traceability.json"),
            "--ideal-map",
            &drifted,
        ],
    );
    std::fs::remove_dir_all(&scratch)?;
    assert_eq!(ran.exit, 1, "{}", ran.report);
    let rules: Vec<&Value> = field(&ran.report, "/findings")
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|finding| field(finding, "/rule"))
        .collect();
    assert_eq!(
        rules,
        [
            &Value::from("predicate-drift"),
            &Value::from("contract-removed")
        ]
    );
    Ok(())
}

#[test]
fn evidence_reports_append_in_invocation_order() -> Result<(), Box<dyn std::error::Error>> {
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("quality-evidence-append");
    std::fs::create_dir_all(&scratch)?;
    let path = scratch.join("task-61.json");
    let text = path.to_string_lossy().into_owned();
    let first = run(
        Tool::SourceCheck,
        &["--fixture", "clean-source", "--evidence", &text],
    );
    let second = run(
        Tool::EvidenceAudit,
        &["--fixture", "fake-exit-zero", "--evidence", &text],
    );
    let written: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    std::fs::remove_dir_all(&scratch)?;
    assert_eq!((first.exit, second.exit), (0, 1));
    assert_eq!(field(&written, "/0/gate"), "source-check");
    assert_eq!(field(&written, "/1/gate"), "evidence-audit");
    assert_eq!(field(&written, "/1/kind"), "evidence_invalid");
    assert_eq!(field(&written, "/2"), &Value::Null);
    Ok(())
}

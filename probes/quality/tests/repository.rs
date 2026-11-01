//! The repository passes its own gates, and the matrix check notices every kind of drift in
//! what it guards: a weakened predicate, an unowned row, and a contract entry that is
//! missing, removed or changed.

pub mod support;

use std::path::{Path, PathBuf};

use ovis_terminal_probes::quality::{GateRun, Tool};
use serde_json::Value;

use self::support::{field, run};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn repo_path(relative: &str) -> String {
    repo().join(relative).to_string_lossy().into_owned()
}

fn checked_in(relative: &str) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&std::fs::read(
        repo().join(relative),
    )?)?)
}

/// `quality-matrix --mode verify` over copies of the two matrices.
fn verify_copies(
    name: &str,
    (trace, map): (&Value, &Value),
) -> Result<GateRun, Box<dyn std::error::Error>> {
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&scratch)?;
    let (trace_path, map_path) = (
        scratch.join("traceability.json"),
        scratch.join("ideal-map.json"),
    );
    std::fs::write(&trace_path, serde_json::to_vec(trace)?)?;
    std::fs::write(&map_path, serde_json::to_vec(map)?)?;
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
            &trace_path.to_string_lossy(),
            "--ideal-map",
            &map_path.to_string_lossy(),
        ],
    );
    std::fs::remove_dir_all(&scratch)?;
    Ok(ran)
}

fn rules(ran: &GateRun) -> Vec<(String, String)> {
    let findings = field(&ran.report, "/findings")
        .as_array()
        .cloned()
        .unwrap_or_default();
    findings
        .iter()
        .map(|finding| {
            let text = |key: &str| field(finding, key).as_str().unwrap_or_default().to_owned();
            (text("/rule"), text("/location"))
        })
        .collect()
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
fn the_checked_in_matrices_verify_against_the_ledgers_and_the_schema() -> TestResult {
    let ran = verify_copies(
        "matrix-clean",
        (
            &checked_in("conformance/traceability.json")?,
            &checked_in("conformance/ideal-map.json")?,
        ),
    )?;
    assert_eq!(ran.exit, 0, "{}", ran.report);
    assert_eq!(field(&ran.report, "/ideals"), "121");
    let map = checked_in("conformance/ideal-map.json")?;
    let removal = field(&map, "/contract/methods/worktree.remove");
    assert!(
        field(removal, "/params").is_array() && field(removal, "/result").is_object(),
        "{removal}"
    );
    assert!(
        field(removal, "/errors")
            .as_array()
            .is_some_and(|errors| !errors.is_empty())
    );
    Ok(())
}

#[test]
fn a_method_missing_from_the_recorded_contract_fails_verify() -> TestResult {
    let mut map = checked_in("conformance/ideal-map.json")?;
    if let Some(Value::Object(methods)) = map.pointer_mut("/contract/methods") {
        methods.remove("worktree.remove");
    }
    let ran = verify_copies(
        "matrix-missing-method",
        (&checked_in("conformance/traceability.json")?, &map),
    )?;
    assert_eq!(ran.exit, 1, "{}", ran.report);
    assert_eq!(
        rules(&ran),
        [(
            "contract-missing".to_owned(),
            "methods/worktree.remove".to_owned()
        )]
    );
    Ok(())
}

#[test]
fn a_removed_or_changed_contract_entry_and_a_weakened_predicate_fail_verify() -> TestResult {
    let mut map = checked_in("conformance/ideal-map.json")?;
    if let Some(predicate) = map.pointer_mut("/ideals/IDEAL-008/predicate") {
        *predicate = Value::from("A weaker statement.");
    }
    if let Some(Value::Object(methods)) = map.pointer_mut("/contract/methods") {
        methods.insert(
            "removed.method".to_owned(),
            Value::Object(serde_json::Map::new()),
        );
    }
    if let Some(params) = map.pointer_mut("/contract/methods/doc.set/params") {
        *params = Value::Array(Vec::new());
    }
    let ran = verify_copies(
        "matrix-drift",
        (&checked_in("conformance/traceability.json")?, &map),
    )?;
    assert_eq!(ran.exit, 1, "{}", ran.report);
    let found = rules(&ran);
    for expected in [
        ("predicate-drift", "IDEAL-008"),
        ("contract-drift", "methods/doc.set"),
        ("contract-removed", "methods/removed.method"),
    ] {
        assert!(
            found.contains(&(expected.0.to_owned(), expected.1.to_owned())),
            "{expected:?} not in {found:?}"
        );
    }
    assert_eq!(found.len(), 3, "{found:?}");
    Ok(())
}

#[test]
fn an_emptied_owner_or_scenario_fails_verify() -> TestResult {
    let mut trace = checked_in("conformance/traceability.json")?;
    if let Some(todos) = trace.pointer_mut("/gaps/GAP-014/todos") {
        *todos = Value::Array(Vec::new());
    }
    let mut map = checked_in("conformance/ideal-map.json")?;
    if let Some(scenarios) = map.pointer_mut("/exclusions/EXCL-029/scenarios") {
        *scenarios = Value::Array(Vec::new());
    }
    let ran = verify_copies("matrix-unowned", (&trace, &map))?;
    assert_eq!(ran.exit, 1, "{}", ran.report);
    assert_eq!(
        rules(&ran),
        [
            (
                "matrix-unowned".to_owned(),
                "traceability/gaps/GAP-014".to_owned()
            ),
            (
                "matrix-unowned".to_owned(),
                "ideal-map/exclusions/EXCL-029".to_owned()
            ),
        ]
    );
    Ok(())
}

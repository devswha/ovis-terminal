//! The committed evidence tree passes evidence-audit; its exemptions are exactly the files
//! the audit rejects, and a stale exemption fails. Reports append in invocation order.

pub mod support;

use std::path::{Path, PathBuf};

use ovis_terminal_probes::quality::Tool;
use serde_json::{Value, json};

use self::support::{field, run};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn exempted_paths() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let list: Value = serde_json::from_slice(&std::fs::read(
        repo().join("probes/quality/evidence-exemptions.json"),
    )?)?;
    let entries = field(&list, "/entries")
        .as_array()
        .cloned()
        .unwrap_or_default();
    Ok(entries
        .iter()
        .filter_map(|entry| field(entry, "/path").as_str().map(str::to_owned))
        .collect())
}

fn audit_tree(exemptions: Option<&str>) -> ovis_terminal_probes::quality::GateRun {
    let root = repo().to_string_lossy().into_owned();
    let mut args = vec!["--repo", root.as_str(), "--tree", "docs/evidence"];
    if let Some(path) = exemptions {
        args.extend(["--exemptions", path]);
    }
    run(Tool::EvidenceAudit, &args)
}

#[test]
fn the_committed_evidence_passes_with_its_listed_exemptions() -> TestResult {
    let ran = audit_tree(Some("probes/quality/evidence-exemptions.json"));
    assert_eq!(ran.exit, 0, "{}", ran.report);
    assert_eq!(
        field(&ran.report, "/exempted").as_array().map(Vec::len),
        Some(exempted_paths()?.len())
    );
    Ok(())
}

#[test]
fn without_exemptions_exactly_the_exempted_files_are_rejected() -> TestResult {
    let ran = audit_tree(None);
    assert_eq!(ran.exit, 1);
    let findings = field(&ran.report, "/findings")
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut rejected: Vec<String> = findings
        .iter()
        .filter_map(|finding| field(finding, "/location").as_str())
        .map(|location| location.split('#').next().unwrap_or(location).to_owned())
        .collect();
    rejected.sort();
    rejected.dedup();
    let mut exempted = exempted_paths()?;
    exempted.sort();
    assert_eq!(rejected, exempted);
    Ok(())
}

#[test]
fn an_exemption_for_valid_evidence_is_stale() -> TestResult {
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("evidence-stale-exemption");
    std::fs::create_dir_all(scratch.join("tree"))?;
    std::fs::write(
        scratch.join("tree/clean.json"),
        include_str!("../fixtures/evidence/clean-evidence.json"),
    )?;
    let document = json!({"format": "ovis-evidence-exemptions/1", "entries": [{"path": "tree/clean.json", "reason": "seeded stale entry"}]});
    std::fs::write(
        scratch.join("exemptions.json"),
        serde_json::to_vec(&document)?,
    )?;
    let root = scratch.to_string_lossy().into_owned();
    let ran = run(
        Tool::EvidenceAudit,
        &[
            "--repo",
            &root,
            "--tree",
            "tree",
            "--exemptions",
            "exemptions.json",
        ],
    );
    std::fs::remove_dir_all(&scratch)?;
    assert_eq!(ran.exit, 1, "{}", ran.report);
    let rules: Vec<(&Value, &Value)> = field(&ran.report, "/findings")
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|finding| (field(finding, "/rule"), field(finding, "/location")))
        .collect();
    assert_eq!(
        rules,
        [(
            &Value::from("stale-exemption"),
            &Value::from("tree/clean.json")
        )]
    );
    Ok(())
}

#[test]
fn evidence_reports_append_in_invocation_order() -> TestResult {
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
    let audited = run(Tool::EvidenceAudit, &["--input", &text]);
    let written: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    std::fs::remove_dir_all(&scratch)?;
    assert_eq!((first.exit, second.exit), (0, 1));
    assert_eq!(field(&written, "/0/gate"), "source-check");
    assert_eq!(field(&written, "/1/kind"), "evidence_invalid");
    assert_eq!(field(&written, "/2"), &Value::Null);
    assert_eq!(
        audited.exit, 0,
        "appended gate reports are audited, not rejected: {}",
        audited.report
    );
    Ok(())
}

//! Auditing a quality-gate report appended to an evidence file: its verdict, exit status,
//! kind and findings must agree, so a report cannot claim a pass its findings contradict.

use serde_json::Value;

use super::cli::Tool;
use super::report::Finding;

/// Every inconsistency in one gate report, reported under `label`.
pub(super) fn audit(label: &str, report: &Value) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut flag =
        |rule: &'static str, detail: String| found.push(Finding::new(rule, label, detail));
    let text = |pointer: &str| report.pointer(pointer).and_then(Value::as_str);
    let gate = text("/gate").unwrap_or_default();
    if Tool::parse(gate).is_none() {
        flag(
            "manifest-schema",
            format!("/gate {gate:?} is not a quality tool"),
        );
    }
    if text("/host/os").is_none() || text("/host/arch").is_none() {
        flag(
            "host",
            "the report does not record the host os and arch".to_owned(),
        );
    }
    let argv_gate = report.pointer("/argv/0").and_then(Value::as_str);
    if argv_gate != Some(gate) {
        flag(
            "manifest-schema",
            "/argv does not start with the gate that ran".to_owned(),
        );
    }
    let findings = report.get("findings").and_then(Value::as_array);
    let Some(findings) = findings else {
        flag("manifest-schema", "/findings is not an array".to_owned());
        return found;
    };
    let counted = text("/finding_count").and_then(|count| count.parse::<usize>().ok());
    if counted != Some(findings.len()) {
        flag(
            "gate-report-forged",
            format!("finding_count {counted:?} but {} findings", findings.len()),
        );
    }
    let clean = findings.is_empty();
    let consistent = match (text("/verdict"), report.get("exit").and_then(Value::as_u64)) {
        (Some("pass"), Some(0)) => clean && report.get("kind").is_some_and(Value::is_null),
        (Some("fail"), Some(1)) => !clean && text("/kind").is_some(),
        _ => false,
    };
    if !consistent {
        flag(
            "gate-report-forged",
            "verdict, exit, kind and findings disagree".to_owned(),
        );
    }
    found
}

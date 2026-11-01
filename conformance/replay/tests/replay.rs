//! `replay` over a scripted transport, and the committed fixture set's own audit.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::path::Path;

use ovis_terminal_replay::{
    Bindings, Case, ConnLabel, FixtureSet, Options, ScenarioState, Transport, replay,
};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Debug, thiserror::Error)]
#[error("the script has no reply left")]
struct Exhausted;

/// Answers each call with the next scripted reply and records what was sent.
#[derive(Default)]
struct Script {
    replies: VecDeque<Value>,
    events: VecDeque<Value>,
    sent: Vec<(String, Value)>,
}

impl Transport for Script {
    type Error = Exhausted;

    fn call(
        &mut self,
        conn: &ConnLabel,
        request: Value,
    ) -> impl Future<Output = Result<Value, Exhausted>> + Send {
        self.sent.push((conn.as_str().to_owned(), request));
        std::future::ready(self.replies.pop_front().ok_or(Exhausted))
    }

    fn event(
        &mut self,
        _conn: &ConnLabel,
        _method: &str,
    ) -> impl Future<Output = Result<Value, Exhausted>> + Send {
        std::future::ready(self.events.pop_front().ok_or(Exhausted))
    }
}

fn repo() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."))
}

fn fixtures() -> Result<FixtureSet, Box<dyn std::error::Error>> {
    Ok(FixtureSet::load(&repo().join("conformance/fixtures/ovis"))?)
}

fn case(steps: &Value, kind: &str) -> Result<Case, serde_json::Error> {
    serde_json::from_value(
        json!({"format": "ovis-replay/1", "id": "k", "server": "assembled",
        "kind": kind, "ideals": [], "gaps": [], "excludes": [], "adversarial": [], "steps": steps}),
    )
}

#[tokio::test]
async fn captures_bind_later_requests_and_a_failed_step_stops_the_case() -> TestResult {
    let set = fixtures()?;
    let case = case(
        &json!([
            {"call": {"conn": "desk", "id": "a", "method": "layout.get", "params": {},
                      "expect": {"/result/revision": {"$capture": "rev"}}}},
            {"call": {"conn": "phone", "id": "b", "method": "layout.edit",
                      "params": {"op_id": "$op:b", "expect_revision": "$value:rev"},
                      "expect": {"/error/data/kind": "revision_mismatch"}}},
            {"call": {"conn": "desk", "id": "c", "method": "layout.get", "params": {}, "expect": {}}}
        ]),
        "happy",
    )?;
    let mut script = Script::default();
    script.replies.extend([
        json!({"result": {"revision": "7"}}),
        json!({"result": {"changed": true}}),
    ]);
    let options = Options::new(&set.legacy, Bindings::default(), None);
    let run = replay(&case, &mut script, options).await?;
    assert_eq!(
        script.sent.len(),
        2,
        "the step after the failed one is not sent"
    );
    assert_eq!(
        script.sent.get(1).map(|(conn, _)| conn.as_str()),
        Some("phone")
    );
    let second = script
        .sent
        .get(1)
        .map(|(_, request)| request.clone())
        .unwrap_or_default();
    assert_eq!(second.pointer("/params/expect_revision"), Some(&json!("7")));
    let op = second
        .pointer("/params/op_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert_eq!(uuid::Uuid::parse_str(op).map(|id| id.get_version_num())?, 7);
    let failed: Vec<&str> = run
        .checks
        .iter()
        .filter(|c| !c.passed)
        .map(|c| c.check.as_str())
        .collect();
    assert_eq!(failed, ["k/b/error/data/kind"]);
    assert!(!run.passed());
    Ok(())
}

#[tokio::test]
async fn an_inject_runs_only_the_planned_step_and_legacy_calls_are_translated() -> TestResult {
    let set = fixtures()?;
    let typo = json!({"pane_id_typo": "other-pane", "direction": "right"});
    let case = case(
        &json!([
            {"legacy": {"conn": "desk", "id": "typo", "method": "pane.split", "params": typo, "inject": true,
                        "expect": {"/error/data/field_path": "params.pane_id_typo"}}},
            {"call": {"conn": "desk", "id": "other", "method": "terminal.process_info", "params": {"pane_typo": "p2"},
                      "inject": true, "expect": {}}}
        ]),
        "failure",
    )?;
    let injected = json!({"method": "pane.split", "params": typo});
    assert!(case.plans(&injected));
    assert!(!case.plans(&json!({"method": "pane.split", "params": {"pane_id": "p"}})));
    let mut script = Script::default();
    script.replies.push_back(
        json!({"error": {"data": {"kind": "invalid_params", "field_path": "params.pane_id_typo"}}}),
    );
    let run = replay(
        &case,
        &mut script,
        Options::new(&set.legacy, Bindings::default(), Some(&injected)),
    )
    .await?;
    assert_eq!(script.sent.len(), 1);
    let request = script
        .sent
        .first()
        .map(|(_, request)| request.clone())
        .unwrap_or_default();
    assert_eq!(request.pointer("/method"), Some(&json!("layout.edit")));
    assert_eq!(
        request.pointer("/params/edit"),
        Some(&json!({"op": "split_pane", "axis": "horizontal"}))
    );
    assert_eq!(
        request.pointer("/params/pane_id_typo"),
        Some(&json!("other-pane"))
    );
    assert!(run.passed());
    assert_eq!(run.receipt.len(), 1);
    Ok(())
}

#[test]
fn the_committed_fixture_set_passes_its_own_audit() -> TestResult {
    let set = fixtures()?;
    let scenarios = repo().join("conformance/scenarios");
    let resolve = |id: &str| {
        if scenarios.join(format!("{id}.json")).is_file() {
            ScenarioState::Executable
        } else {
            ScenarioState::Absent
        }
    };
    let failed: Vec<String> = set
        .audit(repo(), &resolve)?
        .into_iter()
        .filter(|c| !c.passed)
        .map(|c| c.check)
        .collect();
    assert_eq!(failed, Vec::<String>::new());
    assert_eq!(set.predicates.ideals.len(), 121);
    assert_eq!(set.cases.len(), set.receipts.len());
    let bindings: BTreeMap<String, Value> = set
        .manifest
        .bindings((ovis_terminal_replay::Server::Protocol, repo()))?;
    assert_eq!(
        bindings.get("process_token"),
        Some(&json!("fixture-process-token-agent"))
    );
    Ok(())
}

//! Typed conversation records reported by the spawned program are appended and read back
//! through the served binary; a forged process token appends nothing.

use serde_json::{Value, json};

use super::prompts::spawned_token;
use super::support::{Served, TestResult, op_id};

fn t1() -> Value {
    json!({"address": "t1"})
}

fn call(served: &Served, method: &str, params: &Value) -> TestResult<Value> {
    served.ok(&["call", method, "--params", &params.to_string()])
}

fn text(record_id: &str, role: &str, words: &str) -> Value {
    json!({"record_id": record_id, "role": role, "blocks": [{"type": "text", "text": words}]})
}

#[test]
fn records_the_spawned_program_appends_are_read_back_in_order_and_a_forged_token_appends_nothing()
-> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let token = spawned_token(&served)?;
    call(
        &served,
        "agent.register",
        &json!({"op_id": op_id()?, "target": t1(), "process_token": token, "label": "agent", "session": null}),
    )?;
    let working = call(
        &served,
        "status.report",
        &json!({"op_id": op_id()?, "target": t1(), "process_token": token, "seq": "1", "record": {"state": "working"}}),
    )?;
    assert_eq!(
        working.pointer("/status/turn"),
        Some(&json!("1")),
        "{working}"
    );
    let forged = served.cli(&[
        "call",
        "agent.append_records",
        "--params",
        &json!({"op_id": op_id()?, "target": t1(), "process_token": "forged", "records": [text("x1", "user", "no")]})
            .to_string(),
    ])?;
    let refused: Value = serde_json::from_slice(&forged.stderr)?;
    assert_eq!(
        refused.pointer("/error/kind"),
        Some(&json!("not_owner")),
        "{refused}"
    );
    let appended = call(
        &served,
        "agent.append_records",
        &json!({"op_id": op_id()?, "target": t1(), "process_token": token,
            "records": [text("u1", "user", "fix the build"), text("a1", "assistant", "on it")]}),
    )?;
    assert_eq!(
        appended.get("record_ids"),
        Some(&json!(["u1", "a1"])),
        "{appended}"
    );
    assert_eq!(appended.get("total_records"), Some(&json!("2")));
    let read = call(&served, "agent.records", &json!({"target": t1()}))?;
    let ids: Vec<&Value> = read
        .get("records")
        .and_then(Value::as_array)
        .ok_or_else(|| read.to_string())?
        .iter()
        .filter_map(|record| record.get("record_id"))
        .collect();
    assert_eq!(ids, [&json!("u1"), &json!("a1")], "{read}");
    assert_eq!(read.pointer("/records/0/turn"), Some(&json!("1")));
    served.stop()?;
    Ok(())
}

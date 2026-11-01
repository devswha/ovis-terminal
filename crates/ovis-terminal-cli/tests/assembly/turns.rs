//! `turn.get`, `turn.wait`, `events.wait` and `input.interrupt` on the real binary, driven by
//! the program's own declared status: a `working` report satisfies a state wait, an `idle`
//! report ends the open turn a turn wait is bound to, and an interrupt answers with the
//! receipt of the key it wrote.

use serde_json::{Value, json};

use super::prompts::spawned_token;
use super::support::{Served, TestResult, op_id};

fn call(served: &Served, method: &str, params: &Value) -> TestResult<Value> {
    served.ok(&["call", method, "--params", &params.to_string()])
}

fn t1() -> Value {
    json!({"address": "t1"})
}

fn report(served: &Served, token: &str, (state, seq): (&str, &str)) -> TestResult<Value> {
    call(
        served,
        "status.report",
        &json!({"op_id": op_id()?, "target": t1(), "process_token": token, "seq": seq,
            "record": {"state": state, "id": null, "kind": null, "progress": null,
                "app": null, "title": null, "msg": null}}),
    )
}

#[test]
fn declared_status_drives_state_and_turn_waits_and_an_interrupt_reports_its_key() -> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let token = spawned_token(&served)?;
    call(
        &served,
        "agent.register",
        &json!({"op_id": op_id()?, "target": t1(), "process_token": token, "label": "agent",
            "session": null}),
    )?;
    report(&served, &token, ("working", "1"))?;
    let wait = json!({"target": t1(), "states": ["working"], "turn_after": null, "exit": null,
        "cursor": null, "timeout_ms": "10000"});
    let satisfied = call(&served, "events.wait", &wait)?;
    assert_eq!(
        satisfied.get("outcome"),
        Some(&json!("already_satisfied")),
        "{satisfied}"
    );
    assert_ne!(satisfied.get("seq"), Some(&json!("0")), "{satisfied}");

    let open = call(&served, "turn.get", &json!({"target": t1()}))?;
    assert_eq!(open.get("outcome"), Some(&Value::Null), "{open}");
    let turn = open.get("turn").cloned().ok_or_else(|| open.to_string())?;
    let ended = std::thread::scope(|scope| -> TestResult<Value> {
        let waiting = scope.spawn(|| {
            call(
                &served,
                "turn.wait",
                &json!({"target": t1(), "send_op": null, "turn": turn, "timeout_ms": "10000"}),
            )
            .map_err(|error| error.to_string())
        });
        report(&served, &token, ("idle", "2"))?;
        Ok(waiting.join().map_err(|_| "the turn wait panicked")??)
    })?;
    assert_eq!(ended.get("outcome"), Some(&json!("completed")), "{ended}");
    let closed = call(&served, "turn.get", &json!({"target": t1()}))?;
    assert_eq!(closed.get("outcome"), Some(&json!("completed")), "{closed}");

    let unsupported = served.refused(
        &[
            "call",
            "events.wait",
            "--params",
            &json!({"target": t1(), "states": ["idle"], "turn_after": "1", "exit": null,
                "cursor": null, "timeout_ms": "100"})
            .to_string(),
        ],
        1,
    )?;
    assert_eq!(
        unsupported.get("kind"),
        Some(&json!("unsupported_field")),
        "{unsupported}"
    );

    let op = op_id()?;
    let interrupted = call(
        &served,
        "input.interrupt",
        &json!({"op_id": op, "target": t1(), "expect": null, "key": null, "observe_ms": "300",
            "lease_id": null}),
    )?;
    assert_eq!(
        interrupted.pointer("/receipt/op_id"),
        Some(&json!(op)),
        "{interrupted}"
    );
    assert!(
        interrupted.get("stopped").is_some_and(Value::is_boolean),
        "{interrupted}"
    );
    served.stop()?;
    Ok(())
}

#[test]
fn a_status_report_racing_a_state_wait_settles_it_with_the_reported_seq() -> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let token = spawned_token(&served)?;
    call(
        &served,
        "agent.register",
        &json!({"op_id": op_id()?, "target": t1(), "process_token": token, "label": "agent",
            "session": null}),
    )?;
    let wait = json!({"target": t1(), "states": ["working"], "turn_after": null, "exit": null,
        "cursor": null, "timeout_ms": "10000"});
    let waited = std::thread::scope(|scope| -> TestResult<Value> {
        let waiting =
            scope.spawn(|| call(&served, "events.wait", &wait).map_err(|e| e.to_string()));
        report(&served, &token, ("working", "1"))?;
        Ok(waiting.join().map_err(|_| "the wait panicked")??)
    })?;
    // Either the wait was armed first (matched) or the report applied first; both carry it.
    let outcome = waited.get("outcome").and_then(Value::as_str);
    assert!(
        matches!(outcome, Some("matched" | "already_satisfied")),
        "{waited}"
    );
    assert_ne!(waited.get("seq"), Some(&json!("0")), "{waited}");
    served.stop()?;
    Ok(())
}

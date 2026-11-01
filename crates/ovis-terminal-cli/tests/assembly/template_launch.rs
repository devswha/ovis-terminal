//! A template launch runs on the real binary through T35's lane: `terminal.run` of a
//! pinned template with a first message spawns the template's argv and answers the message
//! `accepted`; a `working` declaration does not release it; `idle` releases it once, the
//! program receives it, and `terminal.first_message` reports it `sent`.

use serde_json::{Value, json};

use super::prompts::spawned_token;
use super::support::{Served, TestResult, op_id};

fn call(served: &Served, method: &str, params: &Value) -> TestResult<Value> {
    served.ok(&["call", method, "--params", &params.to_string()])
}

fn declare(served: &Served, token: &str, (state, seq): (&str, &str)) -> TestResult<Value> {
    call(
        served,
        "status.report",
        &json!({"op_id": op_id()?, "target": {"address": "t1"}, "process_token": token,
            "seq": seq, "record": {"state": state, "id": null, "kind": null, "progress": null,
                "app": null, "title": null, "msg": null}}),
    )
}

fn first_message(served: &Served, launch: &str, settled_within_ms: &Value) -> TestResult<Value> {
    call(
        served,
        "terminal.first_message",
        &json!({"launch_op": launch, "settled_within_ms": settled_within_ms}),
    )
}

#[test]
fn a_template_launch_runs_and_its_first_message_is_released_on_idle() -> TestResult {
    let served = Served::start()?;
    let template = json!({"name": "build", "version": "1", "description": "build agent",
        "argv": ["/bin/sh"], "env": {"PS1": "tpl$ ", "PATH": "/usr/bin:/bin"}});
    let dir = served.root().join("state").join("templates");
    std::fs::write(dir.join("build-1.json"), template.to_string())?;
    let launch = op_id()?;
    let launched = call(
        &served,
        "terminal.run",
        &json!({"op_id": launch, "template": {"name": "build", "version": "1"},
            "geometry": {"cols": 80, "rows": 24}, "cwd": "/tmp", "name": "builder",
            "first_message": {"text": "echo released-$((40+2))", "deadline_ms": "10000"}}),
    )?;
    assert_eq!(launched.get("spawn"), Some(&json!("spawned")), "{launched}");
    assert_eq!(
        launched.pointer("/terminal/process/argv"),
        Some(&json!(["/bin/sh"])),
        "{launched}"
    );
    assert_eq!(
        launched.pointer("/first_message/state"),
        Some(&json!("accepted")),
        "{launched}"
    );

    let token = spawned_token(&served)?;
    call(
        &served,
        "agent.register",
        &json!({"op_id": op_id()?, "target": {"address": "t1"}, "process_token": token,
            "label": "builder", "session": null}),
    )?;
    declare(&served, &token, ("working", "1"))?;
    let held = first_message(&served, &launch, &Value::Null)?;
    assert_eq!(held.get("state"), Some(&json!("accepted")), "{held}");

    declare(&served, &token, ("idle", "2"))?;
    let sent = first_message(&served, &launch, &json!("10000"))?;
    assert_eq!(sent.get("state"), Some(&json!("sent")), "{sent}");
    let echoed = call(
        &served,
        "terminal.wait_output",
        &json!({"target": {"address": "t1"}, "cursor": {"offset": "0"},
            "pattern": {"text": "released-42"}, "timeout_ms": "10000"}),
    )?;
    assert_eq!(echoed.get("outcome"), Some(&json!("matched")), "{echoed}");
    served.stop()?;
    Ok(())
}

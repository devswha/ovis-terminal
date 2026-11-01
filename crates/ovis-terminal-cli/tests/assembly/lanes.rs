//! One real-socket smoke test per wired lane, each against a fresh `serve`.

use base64::Engine as _;
use serde_json::{Value, json};

use super::support::{Served, TestResult, op_id};

fn call(served: &Served, method: &str, params: &Value) -> TestResult<Value> {
    served.ok(&["call", method, "--params", &params.to_string()])
}

fn t1() -> Value {
    json!({"address": "t1"})
}

#[test]
fn lifecycle_mutations_label_rename_inspect_kill_and_remove_a_real_program() -> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let labelled = call(
        &served,
        "terminal.label",
        &json!({"op_id": op_id()?, "target": t1(), "set": {"role": "build"}, "remove": null}),
    )?;
    assert_eq!(
        labelled.pointer("/terminal/labels/role"),
        Some(&json!("build"))
    );
    call(
        &served,
        "terminal.rename",
        &json!({"op_id": op_id()?, "target": t1(), "name": "renamed"}),
    )?;
    let by_name = served.ok(&["terminal", "get", "--name", "renamed"])?;
    assert_eq!(by_name.get("address"), Some(&json!("t1")));
    let info = call(&served, "terminal.process_info", &json!({"target": t1()}))?;
    assert_eq!(
        info.pointer("/launcher/pid"),
        by_name.pointer("/process/pid")
    );
    let killed = call(
        &served,
        "terminal.kill",
        &json!({"op_id": op_id()?, "target": t1(), "expect": null, "escalation": {"hup_ms": "500", "term_ms": "500"}}),
    )?;
    assert_eq!(killed.get("observed"), Some(&json!("exited")));
    let after = served.ok(&["terminal", "get", "--address", "t1"])?;
    assert_eq!(after.get("state"), Some(&json!("finished")));
    assert!(
        after
            .pointer("/finished/reason")
            .is_some_and(|reason| reason != &Value::Null)
    );
    call(
        &served,
        "terminal.rm",
        &json!({"op_id": op_id()?, "target": t1()}),
    )?;
    served.stop()?;
    Ok(())
}

#[test]
fn sent_text_is_waited_for_from_its_receipt_cursor_and_read_from_screen_and_output() -> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let sent = call(
        &served,
        "input.send",
        &json!({"op_id": op_id()?, "target": t1(), "expect": null, "text": "echo r-$((2+2))", "submit": "enter", "lease_id": null}),
    )?;
    let cursor = sent
        .pointer("/receipt/output_cursor/offset")
        .cloned()
        .ok_or("receipt cursor")?;
    let waited = call(
        &served,
        "terminal.wait_output",
        &json!({"target": t1(), "cursor": {"offset": cursor}, "pattern": {"text": "r-4"}, "timeout_ms": "10000"}),
    )?;
    assert_eq!(waited.get("outcome"), Some(&json!("matched")), "{waited}");
    let screen = call(
        &served,
        "terminal.screen",
        &json!({"target": t1(), "format": "text"}),
    )?;
    let lines: Vec<&str> = screen
        .get("lines")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    assert!(lines.contains(&"r-4"), "{lines:?}");
    let output = call(
        &served,
        "terminal.output",
        &json!({"target": t1(), "from_offset": "0", "limit_bytes": "65536"}),
    )?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(
        output
            .get("data_b64")
            .and_then(Value::as_str)
            .ok_or("data")?,
    )?;
    assert!(String::from_utf8_lossy(&bytes).contains("r-4"));
    let regex = served.refused(&["call", "terminal.wait_output", "--params", &json!({"target": t1(), "cursor": {"offset": "0"}, "pattern": {"regex": "r-."}, "timeout_ms": "10"}).to_string()], 1)?;
    assert_eq!(regex.get("kind"), Some(&json!("invalid_pattern")));
    served.stop()?;
    Ok(())
}

#[test]
fn keys_and_paste_enter_the_same_sequence_with_increasing_queue_positions() -> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let pasted = call(
        &served,
        "input.paste",
        &json!({"op_id": op_id()?, "target": t1(), "expect": null, "text": "echo p-$((1+5))", "bracketed": "auto", "lease_id": null}),
    )?;
    let keyed = call(
        &served,
        "input.keys",
        &json!({"op_id": op_id()?, "target": t1(), "expect": null, "keys": ["enter"], "lease_id": null}),
    )?;
    let seq = |value: &Value| {
        value
            .pointer("/receipt/queued_seq")
            .and_then(Value::as_str)
            .and_then(|seq| seq.parse::<u64>().ok())
    };
    assert!(seq(&keyed) > seq(&pasted), "{pasted} {keyed}");
    let waited = call(
        &served,
        "terminal.wait_output",
        &json!({"target": t1(), "cursor": {"offset": "0"}, "pattern": {"text": "p-6"}, "timeout_ms": "10000"}),
    )?;
    assert_eq!(waited.get("outcome"), Some(&json!("matched")), "{waited}");
    served.stop()?;
    Ok(())
}

#[test]
fn hosted_lanes_answer_documents_plugins_prompts_and_status_over_the_socket() -> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let set = call(
        &served,
        "doc.set",
        &json!({"op_id": op_id()?, "name": "notes", "value": "kept", "version": "0"}),
    )?;
    assert_eq!(set.get("version"), Some(&json!("1")));
    let got = call(&served, "doc.get", &json!({"name": "notes"}))?;
    assert_eq!(
        (got.get("value"), got.get("version")),
        (Some(&json!("kept")), Some(&json!("1")))
    );
    let stale = served.refused(
        &[
            "call",
            "doc.set",
            "--params",
            &json!({"op_id": op_id()?, "name": "notes", "value": "lost", "version": "0"})
                .to_string(),
        ],
        11,
    )?;
    assert_eq!(stale.get("kind"), Some(&json!("conflict")));
    assert_eq!(
        call(&served, "plugin.list", &json!({}))?,
        json!({"plugins": []})
    );
    assert_eq!(
        call(&served, "prompt.list", &json!({}))?.get("prompts"),
        Some(&json!([]))
    );
    let status = call(&served, "status.get", &json!({"target": t1()}))?;
    assert_eq!(status.pointer("/status/reported"), Some(&json!(false)));
    served.stop()?;
    Ok(())
}

#[test]
fn revoking_the_observer_credential_refuses_its_next_hello() -> TestResult {
    let served = Served::start()?;
    let before = served.hello_reply(served.observer())?;
    assert!(before.get("result").is_some(), "{before}");
    let revoked = call(
        &served,
        "credential.revoke",
        &json!({"op_id": op_id()?, "credential": "c2"}),
    )?;
    assert!(revoked.is_object(), "{revoked}");
    let after = served.hello_reply(served.observer())?;
    assert_eq!(
        after.pointer("/error/data/kind"),
        Some(&json!("credential_revoked")),
        "{after}"
    );
    served.stop()?;
    Ok(())
}

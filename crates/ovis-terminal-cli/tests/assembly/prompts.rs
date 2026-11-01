//! A program finds its process token at spawn, opens a prompt with it, and the prompt is
//! still known to the next boot of the same state.

use serde_json::{Value, json};

use super::support::{Served, TestResult, op_id};

fn t1() -> Value {
    json!({"address": "t1"})
}

fn call(served: &Served, method: &str, params: &Value) -> TestResult<Value> {
    served.ok(&["call", method, "--params", &params.to_string()])
}

/// The token the shell running as `t1` was spawned with, printed by the shell itself.
fn spawned_token(served: &Served) -> TestResult<String> {
    let text = r#"printf 'tok=%s=\n' "$OVIS_TERMINAL_PROCESS_TOKEN"; echo done-$((1+1))"#;
    let sent = call(
        served,
        "input.send",
        &json!({"op_id": op_id()?, "target": t1(), "expect": null, "text": text, "submit": "enter", "lease_id": null}),
    )?;
    let cursor = sent
        .pointer("/receipt/output_cursor/offset")
        .cloned()
        .ok_or("receipt cursor")?;
    let waited = call(
        served,
        "terminal.wait_output",
        &json!({"target": t1(), "cursor": {"offset": cursor}, "pattern": {"text": "done-2"}, "timeout_ms": "10000"}),
    )?;
    assert_eq!(waited.get("outcome"), Some(&json!("matched")), "{waited}");
    let screen = call(
        served,
        "terminal.screen",
        &json!({"target": t1(), "format": "text"}),
    )?;
    let token = screen
        .get("lines")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find_map(|line| line.strip_prefix("tok=")?.strip_suffix('='))
        .filter(|token| !token.is_empty() && !token.contains('%'))
        .ok_or_else(|| screen.to_string())?;
    Ok(token.to_owned())
}

fn prompt_ids(served: &Served) -> TestResult<Vec<Value>> {
    let listed = call(served, "prompt.list", &json!({}))?;
    Ok(listed
        .get("prompts")
        .and_then(Value::as_array)
        .ok_or_else(|| listed.to_string())?
        .iter()
        .filter_map(|prompt| prompt.get("prompt_id").cloned())
        .collect())
}

#[test]
fn a_prompt_opened_with_the_spawn_token_is_recovered_by_the_next_boot() -> TestResult {
    let first = Served::start()?;
    first.run_shell("shell")?;
    let token = spawned_token(&first)?;
    let registered = call(
        &first,
        "agent.register",
        &json!({"op_id": op_id()?, "target": t1(), "process_token": token, "label": "agent", "session": null}),
    )?;
    assert!(
        registered.pointer("/agent/agent_id").is_some(),
        "{registered}"
    );
    let choices = [
        json!({"id": "yes", "label": "Yes"}),
        json!({"id": "no", "label": "No"}),
    ];
    let opened = call(
        &first,
        "prompt.open",
        &json!({
            "op_id": op_id()?, "target": t1(), "process_token": token, "prompt_id": "p1",
            "prompt": {"kind": "permission", "text": "Proceed?", "choices": choices, "mechanism": "rpc"},
        }),
    )?;
    assert_eq!(opened.get("state"), Some(&json!("open")), "{opened}");
    let forged = first.refused(
        &[
            "call",
            "prompt.open",
            "--params",
            &json!({
                "op_id": op_id()?, "target": t1(), "process_token": "not-the-token", "prompt_id": "p2",
                "prompt": {"kind": "permission", "text": "?", "choices": choices, "mechanism": "rpc"},
            })
            .to_string(),
        ],
        1,
    )?;
    assert_eq!(forged.get("kind"), Some(&json!("not_owner")), "{forged}");
    assert_eq!(prompt_ids(&first)?, vec![json!("p1")]);
    let (_, root) = first.stop_keeping_state()?;
    let second = Served::start_at(&root, &[])?;
    assert_eq!(prompt_ids(&second)?, vec![json!("p1")]);
    let recovered = call(&second, "prompt.get", &json!({"prompt_id": "p1"}))?;
    assert_eq!(
        recovered.get("prompt_id"),
        Some(&json!("p1")),
        "{recovered}"
    );
    assert_eq!(recovered.get("version"), Some(&json!("1")), "{recovered}");
    second.stop()?;
    Ok(())
}

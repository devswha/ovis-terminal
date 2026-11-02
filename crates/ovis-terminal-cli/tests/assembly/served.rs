//! Published methods P5 serves, on the real binary: `terminal.history` reads retained
//! scrollback, `agent.list` lists registered agents and `server.start` is
//! `already_running` (`events.ack` is in `served_events`).

use serde_json::{Value, json};

use super::prompts::spawned_token;
use super::support::{Served, TestResult, op_id};

fn call(served: &Served, method: &str, params: &Value) -> TestResult<Value> {
    served.ok(&["call", method, "--params", &params.to_string()])
}

fn t1() -> Value {
    json!({"address": "t1"})
}

fn texts(value: &Value) -> Vec<String> {
    value
        .get("lines")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

#[test]
fn terminal_history_reads_retained_scrollback_newest_first_page_and_from_a_line() -> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let text = "i=0; while [ $i -lt 60 ]; do echo h-$i; i=$((i+1)); done; echo end-$((1+1))";
    let sent = call(
        &served,
        "input.send",
        &json!({"op_id": op_id()?, "target": t1(), "expect": null, "text": text, "submit": "enter", "lease_id": null}),
    )?;
    let cursor = sent
        .pointer("/receipt/output_cursor/offset")
        .cloned()
        .ok_or("receipt cursor")?;
    let waited = call(
        &served,
        "terminal.wait_output",
        &json!({"target": t1(), "cursor": {"offset": cursor}, "pattern": {"text": "end-2"}, "timeout_ms": "10000"}),
    )?;
    assert_eq!(waited.get("outcome"), Some(&json!("matched")), "{waited}");

    let all = call(
        &served,
        "terminal.history",
        &json!({"target": t1(), "from_line": "0", "limit": 1000, "format": "text"}),
    )?;
    let lines = texts(&all);
    assert_eq!(all.get("first_line"), Some(&json!("0")), "{all}");
    assert_eq!(all.get("truncated"), Some(&json!(false)), "{all}");
    for n in [0, 1, 20] {
        assert!(lines.contains(&format!("h-{n}")), "h-{n} in {lines:?}");
    }
    let newest = call(
        &served,
        "terminal.history",
        &json!({"target": t1(), "limit": 3, "format": "text"}),
    )?;
    let tail: Vec<String> = lines
        .iter()
        .skip(lines.len().saturating_sub(3))
        .cloned()
        .collect();
    assert_eq!(texts(&newest), tail, "{newest}");
    let first = lines.len().saturating_sub(3).to_string();
    assert_eq!(newest.get("first_line"), Some(&json!(first)), "{newest}");
    let page = call(
        &served,
        "terminal.history",
        &json!({"target": t1(), "from_line": "1", "limit": 2, "format": "text"}),
    )?;
    assert_eq!(
        texts(&page),
        lines.get(1..3).ok_or("two lines")?.to_vec(),
        "{page}"
    );
    assert_eq!(page.get("truncated"), Some(&json!(true)), "{page}");

    let zero = served.refused(
        &[
            "call",
            "terminal.history",
            "--params",
            &json!({"target": t1(), "limit": 0, "format": "text"}).to_string(),
        ],
        2,
    )?;
    assert_eq!(zero.get("kind"), Some(&json!("invalid_params")), "{zero}");
    let numeric = served.refused(
        &[
            "call",
            "terminal.history",
            "--params",
            &json!({"target": t1(), "from_line": 0, "limit": 1, "format": "text"}).to_string(),
        ],
        2,
    )?;
    assert_eq!(
        numeric.get("kind"),
        Some(&json!("invalid_params")),
        "{numeric}"
    );
    served.stop()?;
    Ok(())
}

#[test]
fn agent_list_shows_a_registered_agent_and_server_start_is_already_running() -> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let empty = call(&served, "agent.list", &json!({"page": null}))?;
    assert_eq!(empty.get("agents"), Some(&json!([])), "{empty}");
    assert_eq!(empty.pointer("/page/total"), Some(&json!(0)), "{empty}");
    let token = spawned_token(&served)?;
    let registered = call(
        &served,
        "agent.register",
        &json!({"op_id": op_id()?, "target": t1(), "process_token": token, "label": "lister", "session": null}),
    )?;
    let listed = call(&served, "agent.list", &json!({}))?;
    assert_eq!(
        listed.pointer("/agents/0/label"),
        Some(&json!("lister")),
        "{listed}"
    );
    assert_eq!(
        listed.pointer("/agents/0/agent_id"),
        registered.pointer("/agent/agent_id")
    );
    assert_eq!(listed.pointer("/page/total"), Some(&json!(1)), "{listed}");
    let cut = call(
        &served,
        "agent.list",
        &json!({"page": {"after": null, "limit": 0}}),
    )?;
    assert_eq!(cut.get("agents"), Some(&json!([])), "{cut}");
    assert_eq!(cut.pointer("/page/truncated"), Some(&json!(true)), "{cut}");
    let token_page = served.refused(
        &[
            "call",
            "agent.list",
            "--params",
            &json!({"page": {"after": "x", "limit": null}}).to_string(),
        ],
        2,
    )?;
    assert_eq!(
        token_page.get("kind"),
        Some(&json!("invalid_params")),
        "{token_page}"
    );

    let started = served.refused(
        &[
            "call",
            "server.start",
            "--params",
            &json!({"op_id": op_id()?}).to_string(),
        ],
        11,
    )?;
    assert_eq!(
        started.get("kind"),
        Some(&json!("already_running")),
        "{started}"
    );
    served.stop()?;
    Ok(())
}

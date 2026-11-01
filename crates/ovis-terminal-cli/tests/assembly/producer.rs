//! A synthetic producer reports through the typed verbs with the token it inherited at
//! spawn; the token of another terminal is refused and changes nothing.

use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

use super::support::{BIN, Served, TestResult, op_id};

/// A program that prints its own process token on the first screen line, then waits.
const PRODUCER: &str = r#"printf '%s\nREADY\n' "$OVIS_TERMINAL_PROCESS_TOKEN"; exec cat"#;

fn launch(served: &Served, name: &str) -> TestResult<String> {
    let params = json!({
        "op_id": op_id()?, "program": {"argv": ["/bin/sh", "-c", PRODUCER]},
        "geometry": {"cols": 80, "rows": 24}, "cwd": "/tmp",
        "env": {"PATH": "/usr/bin:/bin", "TERM": "xterm"},
        "name": name, "labels": null, "parent": null, "child_policy": null,
        "keep": null, "first_message": null, "machine": null,
    });
    let ran = served.ok(&["call", "terminal.run", "--params", &params.to_string()])?;
    let address = ran
        .pointer("/terminal/address")
        .and_then(Value::as_str)
        .ok_or_else(|| ran.to_string())?
        .to_owned();
    let target = json!({"address": address});
    let wait = json!({"target": target, "cursor": {"offset": "0"}, "pattern": {"text": "READY"}, "timeout_ms": "10000"});
    let waited = served.ok(&[
        "call",
        "terminal.wait_output",
        "--params",
        &wait.to_string(),
    ])?;
    assert_eq!(waited.get("outcome"), Some(&json!("matched")), "{waited}");
    let screen = json!({"target": target, "format": "text"});
    let read = served.ok(&["call", "terminal.screen", "--params", &screen.to_string()])?;
    let token = read
        .pointer("/lines/0")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| read.to_string())?;
    Ok(token.to_owned())
}

/// Runs `ovis-terminal --json <args>` as the program holding `token` would.
fn as_program(served: &Served, token: &str, args: &[&str]) -> TestResult<Output> {
    let socket = served
        .ready()
        .pointer("/ready/socket")
        .and_then(Value::as_str)
        .ok_or("socket")?;
    Ok(Command::new(BIN)
        .arg("--json")
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("OVIS_TERMINAL_SOCKET", socket)
        .env("OVIS_TERMINAL_CREDENTIAL", served.bearer())
        .env("OVIS_TERMINAL_PROCESS_TOKEN", token)
        .stdin(Stdio::null())
        .output()?)
}

fn report(seq: &str, state: &str) -> String {
    json!({"seq": seq, "record": {"state": state}}).to_string()
}

#[test]
fn a_producer_reports_with_its_inherited_token_and_a_foreign_token_is_refused() -> TestResult {
    let served = Served::start()?;
    let own = launch(&served, "producer")?;
    let foreign = launch(&served, "foreign")?;
    assert_ne!(own, foreign);

    let registered = as_program(
        &served,
        &own,
        &[
            "agent",
            "register",
            "--address",
            "t1",
            "--op-id",
            &op_id()?,
            "--params",
            r#"{"label":"synthetic-producer","session":null}"#,
        ],
    )?;
    assert_eq!(registered.status.code(), Some(0), "{registered:?}");
    let reported = as_program(
        &served,
        &own,
        &[
            "status",
            "report",
            "--address",
            "t1",
            "--op-id",
            &op_id()?,
            "--params",
            &report("1", "working"),
        ],
    )?;
    assert_eq!(reported.status.code(), Some(0), "{reported:?}");

    let refused = as_program(
        &served,
        &foreign,
        &[
            "status",
            "report",
            "--address",
            "t1",
            "--op-id",
            &op_id()?,
            "--params",
            &report("2", "idle"),
        ],
    )?;
    assert_eq!(refused.status.code(), Some(12), "{refused:?}");
    assert_eq!(refused.stdout, b"", "{refused:?}");
    let record: Value = serde_json::from_slice(&refused.stderr)?;
    assert_eq!(
        record.pointer("/error/kind"),
        Some(&json!("not_owner")),
        "{record}"
    );
    assert_eq!(
        record.pointer("/error/origin"),
        Some(&json!("server")),
        "{record}"
    );

    let status = served.ok(&["status", "get", "--address", "t1"])?;
    assert_eq!(
        status.pointer("/status/report_seq"),
        Some(&json!("1")),
        "{status}"
    );
    assert_eq!(
        status.pointer("/status/root/report/state"),
        Some(&json!("working")),
        "{status}"
    );
    let agent = served.ok(&["agent", "get", "--address", "t1"])?;
    assert_eq!(
        agent.pointer("/agent/label"),
        Some(&json!("synthetic-producer")),
        "{agent}"
    );
    let plain = served.ok(&["agent", "get", "--address", "t2"])?;
    assert_eq!(plain.get("agent"), Some(&Value::Null), "{plain}");
    served.stop()?;
    Ok(())
}

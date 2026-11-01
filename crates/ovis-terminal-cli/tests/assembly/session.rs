//! `session.snapshot` on the real binary: the spawned terminal, its agent binding, an
//! attached client's lease and a created workspace appear together, and a page limit
//! below the terminal count is reported truncated.

use serde_json::{Value, json};

use super::prompts::spawned_token;
use super::support::{Served, TestResult, op_id, rpc};

#[test]
fn the_session_snapshot_holds_terminals_agents_leases_and_the_layout() -> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let token = spawned_token(&served)?;
    served.ok(&[
        "call",
        "agent.register",
        "--params",
        &json!({"op_id": op_id()?,
        "target": {"address": "t1"}, "process_token": token, "label": "agent",
        "session": null})
        .to_string(),
    ])?;
    served.ok(&[
        "call",
        "layout.edit",
        "--params",
        &json!({"op_id": op_id()?,
        "expect_revision": null,
        "edit": {"op": "create_workspace", "name": "main", "program": null}})
        .to_string(),
    ])?;
    let mut attached = served.connected()?;
    let opened = rpc(
        &mut attached,
        "attach.open",
        &json!({"op_id": op_id()?,
        "target": {"address": "t1"}, "role": "observe", "client_id": "watcher",
        "geometry": null, "stream_mode": "plain", "takeover": null}),
    )?;
    assert!(opened.get("result").is_some(), "{opened}");

    let whole = served.ok(&["call", "session.snapshot", "--params", "{\"page\": null}"])?;
    let count = |field: &str| whole.get(field).and_then(Value::as_array).map(Vec::len);
    assert_eq!(count("terminals"), Some(1), "{whole}");
    assert_eq!(count("agents"), Some(1), "{whole}");
    assert_eq!(count("leases"), Some(1), "{whole}");
    assert_eq!(
        whole.pointer("/layout/workspaces/0/name"),
        Some(&json!("main")),
        "{whole}"
    );
    assert_eq!(
        whole.pointer("/page/truncated"),
        Some(&json!(false)),
        "{whole}"
    );

    let page = served.ok(&[
        "call",
        "session.snapshot",
        "--params",
        &json!({"page": {"after": null, "limit": 0}}).to_string(),
    ])?;
    assert_eq!(page.get("terminals"), Some(&json!([])), "{page}");
    assert_eq!(
        page.pointer("/page/truncated"),
        Some(&json!(true)),
        "{page}"
    );
    drop(attached);
    served.stop()?;
    Ok(())
}

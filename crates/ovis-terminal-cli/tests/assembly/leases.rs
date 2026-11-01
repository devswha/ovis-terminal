//! `attach.focus` and `attach.release_geometry` on a live terminal: focus is recorded only
//! for a lease the caller holds (another principal's lease is `no_lease`), and only the geometry lease can be released, after which
//! the terminal has no geometry owner while the interact lease stays held.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;

use serde_json::{Value, json};

use super::support::{Served, TestResult, op_id};

/// Sends one request on `connection` and returns its reply, skipping notifications.
fn rpc(connection: &mut BufReader<UnixStream>, method: &str, params: &Value) -> TestResult<Value> {
    let id = op_id()?;
    let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    writeln!(connection.get_mut(), "{request}")?;
    loop {
        let mut line = String::new();
        if connection.read_line(&mut line)? == 0 {
            return Err("the connection closed".into());
        }
        let record: Value = serde_json::from_str(&line)?;
        if record.get("id") == Some(&json!(id)) {
            return Ok(record);
        }
    }
}

fn leases(served: &Served) -> TestResult<Value> {
    served.ok(&[
        "call",
        "attach.leases",
        "--params",
        &json!({"target": {"address": "t1"}}).to_string(),
    ])
}

#[test]
fn focus_needs_a_held_lease_and_only_the_geometry_lease_is_released() -> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let mut held = served.connected()?;
    let opened = rpc(
        &mut held,
        "attach.open",
        &json!({"op_id": op_id()?, "target": {"address": "t1"}, "role": "interact",
            "client_id": "pane", "geometry": {"cols": 80, "rows": 24}, "stream_mode": "plain",
            "takeover": null}),
    )?;
    let interact = opened
        .pointer("/result/lease/lease_id")
        .cloned()
        .ok_or_else(|| opened.to_string())?;
    let before = leases(&served)?;
    let geometry = before
        .pointer("/geometry_owner/lease_id")
        .cloned()
        .ok_or_else(|| before.to_string())?;
    let lease = |lease: &Value| -> TestResult<Value> {
        Ok(json!({"op_id": op_id()?, "target": {"address": "t1"}, "lease_id": lease}))
    };

    let focused = rpc(&mut held, "attach.focus", &lease(&geometry)?)?;
    assert_eq!(
        focused.pointer("/result/geometry_owner/lease_id"),
        Some(&geometry),
        "{focused}"
    );
    assert!(focused.pointer("/result/seq").is_some(), "{focused}");
    let mut other = served.connected_as(served.observer())?;
    let watching = rpc(
        &mut other,
        "attach.open",
        &json!({"op_id": op_id()?, "target": {"address": "t1"}, "role": "observe",
            "client_id": "watcher", "geometry": null, "stream_mode": "plain", "takeover": null}),
    )?;
    let stranger = watching
        .pointer("/result/lease/lease_id")
        .cloned()
        .ok_or_else(|| watching.to_string())?;
    let refused = rpc(&mut held, "attach.focus", &lease(&stranger)?)?;
    assert_eq!(
        refused.pointer("/error/data/kind"),
        Some(&json!("no_lease")),
        "{refused}"
    );

    let wrong = rpc(&mut held, "attach.release_geometry", &lease(&interact)?)?;
    assert_eq!(
        wrong.pointer("/error/data/kind"),
        Some(&json!("no_lease")),
        "{wrong}"
    );
    let released = rpc(&mut held, "attach.release_geometry", &lease(&geometry)?)?;
    assert_eq!(
        released.pointer("/result/geometry_owner"),
        Some(&Value::Null),
        "{released}"
    );
    assert!(released.pointer("/result/seq").is_some(), "{released}");
    let after = leases(&served)?;
    assert_eq!(after.get("geometry_owner"), Some(&Value::Null), "{after}");
    assert_eq!(
        after.pointer("/interact/lease_id"),
        Some(&interact),
        "{after}"
    );
    drop((held, other));
    served.stop()?;
    Ok(())
}

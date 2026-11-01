//! `server.stop`, the planned `server.restart` and crash recovery at start-up against real
//! `serve` processes.

use std::io::{BufRead as _, BufReader};
use std::os::unix::net::UnixStream;

use serde_json::{Value, json};

use super::support::{Served, TestResult, op_id, rpc};

fn stop_params(scope: Option<&str>) -> TestResult<String> {
    let mut params = json!({"op_id": op_id()?});
    if let (Some(scope), Value::Object(fields)) = (scope, &mut params) {
        fields.insert("scope".to_owned(), json!(scope));
    }
    Ok(params.to_string())
}

#[test]
fn server_stop_names_its_scope_ends_the_programs_and_the_server_stops_by_itself() -> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let unscoped = served.refused(&["call", "server.stop", "--params", &stop_params(None)?], 2)?;
    assert_eq!(unscoped.get("kind"), Some(&json!("invalid_params")));
    let stopped = served.ok(&[
        "call",
        "server.stop",
        "--params",
        &stop_params(Some("control_and_programs"))?,
    ])?;
    let ended = stopped
        .get("ended_incarnations")
        .and_then(Value::as_array)
        .ok_or_else(|| stopped.to_string())?;
    assert_eq!(ended.len(), 1, "{stopped}");
    assert_eq!(stopped.get("preserved_incarnations"), Some(&json!([])));
    assert_eq!(stopped.get("server_boot_id_new"), Some(&Value::Null));
    let report = served.stopped_by_itself()?;
    assert_eq!(report.pointer("/report/panicked"), Some(&json!("0")));
    Ok(())
}

#[test]
fn a_boot_after_a_crash_reports_only_the_programs_it_inherited() -> TestResult {
    let first = Served::start()?;
    assert_eq!(first.ready().pointer("/ready/recovery"), None);
    let root = first.crash()?;
    let second = Served::start_at(&root, &[])?;
    assert_eq!(second.ready().pointer("/ready/recovery"), None);
    second.run_shell("shell")?;
    let root = second.crash()?;
    let third = Served::start_at(&root, &[])?;
    let recovery = third
        .ready()
        .pointer("/ready/recovery")
        .cloned()
        .ok_or("the third boot reports no recovery")?;
    assert_eq!(
        recovery.get("cause"),
        Some(&json!("core_crash")),
        "{recovery}"
    );
    let terminals = recovery
        .get("terminals")
        .and_then(Value::as_array)
        .ok_or_else(|| recovery.to_string())?;
    assert_eq!(terminals.len(), 1, "{recovery}");
    let incidents = recovery
        .get("incidents")
        .and_then(Value::as_array)
        .ok_or_else(|| recovery.to_string())?;
    assert!(!incidents.is_empty(), "{recovery}");
    third.stop()?;
    Ok(())
}

#[test]
fn a_clean_stop_ends_the_programs_so_the_next_boot_recovers_nothing() -> TestResult {
    let first = Served::start()?;
    first.run_shell("shell")?;
    let (report, root) = first.stop_keeping_state()?;
    assert_eq!(
        report.pointer("/report/panicked"),
        Some(&json!("0")),
        "{report}"
    );
    let second = Served::start_at(&root, &[])?;
    assert_eq!(second.ready().pointer("/ready/recovery"), None);
    let listed = second.ok(&["call", "terminal.list", "--params", "{}"])?;
    let states: Vec<&Value> = listed
        .get("terminals")
        .and_then(Value::as_array)
        .ok_or_else(|| listed.to_string())?
        .iter()
        .filter_map(|terminal| terminal.get("state"))
        .collect();
    assert_eq!(states.len(), 1, "{listed}");
    assert_ne!(states.first(), Some(&&json!("live")), "{listed}");
    second.stop()?;
    Ok(())
}

/// Reads the records of `connection` until `$/closing`; its read timeout bounds the wait.
///
/// # Errors
///
/// Fails when the connection ends first or a record is not JSON.
pub fn closing(connection: &mut BufReader<UnixStream>) -> TestResult<Value> {
    loop {
        let mut line = String::new();
        if connection.read_line(&mut line)? == 0 {
            return Err("the connection ended without $/closing".into());
        }
        let record: Value = serde_json::from_str(&line)?;
        if record.get("method") == Some(&json!("$/closing")) {
            return Ok(record);
        }
    }
}

/// The program's pid and start token, which together name one incarnation.
///
/// # Errors
///
/// Fails when a process or socket step fails.
///
/// # Panics
///
/// Panics when the binary's behaviour differs from the expectation.
pub fn pid(served: &Served) -> TestResult<Value> {
    let params = json!({"target": {"address": "t1"}}).to_string();
    let info = served.ok(&["call", "terminal.process_info", "--params", &params])?;
    let launcher = info.get("launcher").ok_or_else(|| info.to_string())?;
    Ok(json!([launcher.get("pid"), launcher.get("start_token")]))
}

#[test]
fn a_planned_restart_keeps_the_program_and_its_lifetime_and_serves_the_successor_boot() -> TestResult
{
    let mut served = Served::start()?;
    served.run_shell("shell")?;
    let before = served.hello_reply(served.bearer())?;
    let program = pid(&served)?;
    let mut held = served.connected()?;
    let receipt = served.ok(&[
        "call",
        "server.restart",
        "--params",
        &json!({"op_id": op_id()?}).to_string(),
    ])?;
    let successor = receipt
        .get("server_boot_id_new")
        .cloned()
        .ok_or_else(|| receipt.to_string())?;
    assert_eq!(
        receipt.get("server_boot_id_old"),
        before.pointer("/result/server_boot_id"),
        "{receipt}"
    );
    assert_ne!(receipt.get("server_boot_id_old"), Some(&successor));
    assert_eq!(
        receipt.get("ended_incarnations"),
        Some(&json!([])),
        "{receipt}"
    );
    let preserved = receipt
        .get("preserved_incarnations")
        .and_then(Value::as_array);
    assert_eq!(preserved.map(Vec::len), Some(1), "{receipt}");
    let closed = closing(&mut held)?;
    assert_eq!(
        closed.pointer("/params/reason"),
        Some(&json!("server_restart"))
    );
    assert_eq!(
        closed.pointer("/params/incarnations_preserved"),
        Some(&json!(true))
    );
    let announced = served.announcement()?;
    assert_eq!(
        announced.pointer("/restarted/server_boot_id"),
        Some(&successor),
        "{announced}"
    );
    let after = served.hello_reply(served.bearer())?;
    assert_eq!(
        after.pointer("/result/runtime_lifetime"),
        before.pointer("/result/runtime_lifetime")
    );
    assert_eq!(after.pointer("/result/server_boot_id"), Some(&successor));
    assert_eq!(
        pid(&served)?,
        program,
        "the successor serves the same program"
    );
    let status = served.ok(&["call", "server.status", "--params", "{}"])?;
    assert_eq!(status.get("server_boot_id"), Some(&successor), "{status}");
    let sent = served.ok(&[
        "call",
        "input.send",
        "--params",
        &json!({"op_id": op_id()?, "target": {"address": "t1"}, "expect": null,
            "text": "echo again-$((3+4))", "submit": "enter", "lease_id": null})
        .to_string(),
    ])?;
    let cursor = sent
        .pointer("/receipt/output_cursor/offset")
        .cloned()
        .ok_or_else(|| sent.to_string())?;
    let waited = served.ok(&[
        "call",
        "terminal.wait_output",
        "--params",
        &json!({"target": {"address": "t1"}, "cursor": {"offset": cursor},
            "pattern": {"text": "again-7"}, "timeout_ms": "10000"})
        .to_string(),
    ])?;
    assert_eq!(waited.get("outcome"), Some(&json!("matched")), "{waited}");
    let report = served.stop()?;
    assert_eq!(report.pointer("/report/panicked"), Some(&json!("0")));
    Ok(())
}

#[test]
fn an_attached_stream_is_closed_server_restart_before_the_connection_closes() -> TestResult {
    let mut served = Served::start()?;
    served.run_shell("shell")?;
    let mut attached = served.connected()?;
    let opened = rpc(
        &mut attached,
        "attach.open",
        &json!({"op_id": op_id()?, "target": {"address": "t1"}, "role": "interact",
            "client_id": "pane", "geometry": null, "stream_mode": "plain", "takeover": null}),
    )?;
    let stream = opened
        .pointer("/result/stream_id")
        .cloned()
        .ok_or_else(|| opened.to_string())?;
    served.ok(&[
        "call",
        "server.restart",
        "--params",
        &json!({"op_id": op_id()?}).to_string(),
    ])?;
    let mut records = Vec::new();
    loop {
        let mut line = String::new();
        if attached.read_line(&mut line)? == 0 {
            break;
        }
        records.push(serde_json::from_str::<Value>(&line)?);
    }
    let position = |found: &dyn Fn(&Value) -> bool| records.iter().position(found);
    let revoked = position(&|record| {
        record.pointer("/params/stream_id") == Some(&stream)
            && record.pointer("/params/kind") == Some(&json!("input_revoked"))
    });
    let closed = position(&|record| {
        record.pointer("/params/stream_id") == Some(&stream)
            && record.pointer("/params/kind") == Some(&json!("close"))
            && record.pointer("/params/close/reason") == Some(&json!("server_restart"))
    });
    let closing = position(&|record| record.get("method") == Some(&json!("$/closing")));
    assert!(
        matches!((revoked, closed, closing), (Some(r), Some(c), Some(z)) if r < c && c < z),
        "input_revoked {revoked:?}, close {closed:?}, $/closing {closing:?}: {records:?}"
    );
    let announced = served.announcement()?;
    assert!(announced.get("restarted").is_some(), "{announced}");
    served.stop()?;
    Ok(())
}

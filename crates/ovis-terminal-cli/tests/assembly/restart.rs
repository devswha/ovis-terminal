//! `server.stop` and crash recovery at start-up against real `serve` processes.

use serde_json::{Value, json};

use super::support::{Served, TestResult, op_id};

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

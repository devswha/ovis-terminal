//! The assembled binary: `ovis-terminal serve` runs real programs, advertises exactly the
//! methods it routes and refuses everything else with a typed error.
#![cfg(unix)]

#[path = "assembly/control.rs"]
mod control;
#[path = "assembly/events.rs"]
mod events;
#[path = "assembly/lanes.rs"]
mod lanes;
#[path = "assembly/layout.rs"]
mod layout;
#[path = "assembly/leases.rs"]
mod leases;
#[path = "assembly/prompts.rs"]
pub mod prompts;
#[path = "assembly/records.rs"]
mod records;
#[path = "assembly/restart.rs"]
pub mod restart;
#[path = "assembly/routes.rs"]
mod routes;
#[path = "assembly/support.rs"]
pub mod support;
#[path = "assembly/templates.rs"]
mod templates;
#[path = "assembly/turns.rs"]
mod turns;
#[path = "assembly/update.rs"]
mod update;
#[path = "assembly/websocket.rs"]
mod websocket;
#[path = "assembly/worktrees.rs"]
mod worktrees;

use std::collections::BTreeSet;
use std::process::Command;

use serde_json::{Value, json};
use support::{BIN, Served, TestResult, op_id};

/// Schema methods no landed crate implements; each must answer `method_not_found`.
const UNIMPLEMENTED: [&str; 18] = [
    "agent.list",
    "agent.note",
    "agent.release",
    "agent.wait_children",
    "doc.list",
    "events.ack",
    "machine.exec",
    "machine.file",
    "machine.list",
    "machine.login",
    "machine.rm",
    "server.start",
    "service.install",
    "service.remove",
    "session.snapshot",
    "terminal.first_message",
    "terminal.history",
    "terminal.search",
];

/// Every method name the committed OpenRPC document publishes.
fn schema_methods() -> TestResult<BTreeSet<String>> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../schema/ovis-terminal.openrpc.json"
    );
    let document: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    Ok(document
        .get("methods")
        .and_then(Value::as_array)
        .ok_or("the document lists no methods")?
        .iter()
        .filter_map(|method| method.get("name").and_then(Value::as_str))
        .map(str::to_owned)
        .collect())
}

#[test]
fn hello_advertises_every_schema_method_but_the_unimplemented_which_are_method_not_found()
-> TestResult {
    let served = Served::start()?;
    let advertised = served.advertised()?;
    let schema = schema_methods()?;
    let unpublished: Vec<&String> = advertised.difference(&schema).collect();
    assert!(
        unpublished.is_empty(),
        "advertised outside the schema: {unpublished:?}"
    );
    let missing: BTreeSet<String> = schema.difference(&advertised).cloned().collect();
    let unimplemented: BTreeSet<String> = UNIMPLEMENTED.map(str::to_owned).into();
    assert_eq!(
        missing, unimplemented,
        "schema methods hello does not advertise"
    );
    for method in UNIMPLEMENTED {
        let error = served.refused(&["call", method, "--params", "{}"], 4)?;
        assert_eq!(
            error.get("kind"),
            Some(&json!("method_not_found")),
            "{method}: {error}"
        );
    }
    let report = served.stop()?;
    assert_eq!(report.pointer("/report/panicked"), Some(&json!("0")));
    Ok(())
}

#[test]
fn every_verb_is_discoverable_in_the_cli_contract_and_the_manual() -> TestResult {
    let contract: Value = serde_json::from_slice(
        &Command::new(BIN)
            .args(["--json", "schema", "cli"])
            .env_clear()
            .output()?
            .stdout,
    )?;
    let commands: Vec<&str> = contract
        .get("commands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("command").and_then(Value::as_str))
        .collect();
    for verb in [
        "serve",
        "hold",
        "connect",
        "schema",
        "terminal attach",
        "server status",
    ] {
        assert!(commands.contains(&verb), "missing {verb}: {commands:?}");
    }
    let manual = Command::new(BIN).arg("--skill").env_clear().output()?;
    let text = String::from_utf8(manual.stdout)?;
    for verb in [
        "ovis-terminal serve",
        "ovis-terminal hold",
        "ovis-terminal terminal attach",
    ] {
        assert!(text.contains(verb), "manual omits {verb}");
    }
    Ok(())
}

#[test]
fn terminal_run_spawns_a_real_program_that_list_get_and_status_report() -> TestResult {
    let served = Served::start()?;
    let run = served.run_shell("shell")?;
    assert_eq!(run.get("spawn"), Some(&json!("spawned")), "{run}");
    let terminal = run.get("terminal").ok_or("terminal")?;
    assert_eq!(terminal.get("address"), Some(&json!("t1")));
    assert_eq!(terminal.get("state"), Some(&json!("live")));
    let pid = terminal
        .pointer("/process/pid")
        .and_then(Value::as_u64)
        .ok_or("pid")?;
    assert!(pid > 1, "a real child pid, got {pid}");
    let got = served.ok(&["terminal", "get", "--address", "t1"])?;
    assert_eq!(got.get("terminal_id"), terminal.get("terminal_id"));
    assert_eq!(
        got.pointer("/process/pid"),
        terminal.pointer("/process/pid")
    );
    let by_name = served.ok(&["terminal", "get", "--name", "shell"])?;
    assert_eq!(by_name.get("terminal_id"), terminal.get("terminal_id"));
    let listed = served.ok(&["terminal", "list"])?;
    assert_eq!(
        listed.pointer("/terminals/0/terminal_id"),
        terminal.get("terminal_id")
    );
    let status = served.ok(&["server", "status"])?;
    assert_eq!(status.get("state"), Some(&json!("running")));
    assert_eq!(status.get("live_terminals"), Some(&json!("1")));
    assert_eq!(status.get("holder_count"), Some(&json!("1")));
    let uptime = status
        .get("uptime_ms")
        .and_then(Value::as_str)
        .ok_or("uptime_ms is a decimal string")?;
    assert_eq!(uptime.parse::<u64>()?.to_string(), uptime);
    served.stop()?;
    Ok(())
}

#[test]
fn stale_targets_unsupported_launch_fields_and_numeric_ids_are_typed_refusals() -> TestResult {
    let served = Served::start()?;
    served.run_shell("only")?;
    let stale = served.refused(&["terminal", "get", "--address", "t9"], 3)?;
    assert_eq!(stale.get("kind"), Some(&json!("not_found")));
    let first_message = json!({
        "op_id": op_id()?, "program": {"argv": ["/bin/true"]}, "geometry": {"cols": 80, "rows": 24},
        "cwd": "/tmp", "env": {}, "name": null, "labels": null, "parent": null, "child_policy": null,
        "keep": null, "first_message": {"text": "hi", "deadline_ms": "1000"}, "machine": null,
    });
    let unsupported = served.refused(
        &[
            "call",
            "terminal.run",
            "--params",
            &first_message.to_string(),
        ],
        1,
    )?;
    assert_eq!(unsupported.get("kind"), Some(&json!("unsupported_field")));
    let numeric = served.refused(&["call", "op.get", "--params", "{\"op_id\":7}"], 2)?;
    assert_eq!(numeric.get("kind"), Some(&json!("invalid_params")));
    let leases = served.ok(&[
        "call",
        "attach.leases",
        "--params",
        "{\"target\":{\"address\":\"t1\"}}",
    ])?;
    assert_eq!(
        leases,
        json!({"interact": null, "geometry_owner": null, "observers": []})
    );
    served.stop()?;
    Ok(())
}

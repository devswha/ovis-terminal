//! The assembled binary: `ovis-terminal serve` runs real programs, advertises exactly the
//! methods it routes and refuses everything else with a typed error.
#![cfg(unix)]

#[path = "assembly/lanes.rs"]
mod lanes;
#[path = "assembly/support.rs"]
pub mod support;

use std::collections::BTreeSet;
use std::process::Command;

use serde_json::{Value, json};
use support::{BIN, Served, TestResult, op_id};

#[test]
fn hello_advertises_exactly_the_routed_methods_and_the_rest_is_method_not_found() -> TestResult {
    let served = Served::start()?;
    let expected: BTreeSet<String> = [
        "agent.conversation",
        "agent.get",
        "agent.register",
        "agent.update_session",
        "attach.close",
        "attach.credit",
        "attach.leases",
        "attach.open",
        "attach.repaint",
        "attach.resize",
        "doc.get",
        "doc.set",
        "input.keys",
        "input.paste",
        "input.raw",
        "input.send",
        "machine.get",
        "machine.rename",
        "op.cancel",
        "op.get",
        "plugin.invoke",
        "plugin.list",
        "plugin.run_get",
        "prompt.answer",
        "prompt.close",
        "prompt.get",
        "prompt.list",
        "prompt.open",
        "server.status",
        "status.clear",
        "status.get",
        "status.report",
        "terminal.get",
        "terminal.keep",
        "terminal.kill",
        "terminal.label",
        "terminal.list",
        "terminal.output",
        "terminal.process_info",
        "terminal.recover",
        "terminal.rename",
        "terminal.rm",
        "terminal.run",
        "terminal.screen",
        "terminal.wait_output",
    ]
    .map(str::to_owned)
    .into();
    assert_eq!(served.advertised()?, expected);
    for unrouted in [
        "layout.get",
        "machine.list",
        "template.list",
        "update.query",
    ] {
        let error = served.refused(&["call", unrouted, "--params", "{}"], 4)?;
        assert_eq!(
            error.get("kind"),
            Some(&json!("method_not_found")),
            "{unrouted}: {error}"
        );
    }
    let layout = served.refused(&["call", "layout.get", "--params", "{}"], 4)?;
    assert_eq!(layout.get("kind"), Some(&json!("method_not_found")));
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
    assert_eq!(status.get("live_terminals"), Some(&json!(1)));
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

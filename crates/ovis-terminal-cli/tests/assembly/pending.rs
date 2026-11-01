//! IDEAL-010 on the real binary: a caller always learns its operation's id by the deadline,
//! even while the runtime is held (a QA hold FIFO the actor opens with the control), and
//! `op.get` then reports how the operation settled. A planned restart answered pending
//! before control restarted is still handed over once control announces it.

use std::path::Path;
use std::process::Command;

use ovis_terminal_protocol::schema::limits::Limits;
use serde_json::{Value, json};

use super::support::{Served, TestResult, fresh_root, op_id};

/// A root whose server holds its next `terminal.run` on a FIFO, and that FIFO.
fn held() -> TestResult<(Served, std::path::PathBuf)> {
    let root = fresh_root()?;
    let fifo = root.join("hold");
    let made = Command::new("/usr/bin/mkfifo").arg(&fifo).status()?;
    assert!(made.success(), "mkfifo {}", fifo.display());
    let fifo_arg = fifo.to_string_lossy().into_owned();
    Ok((
        Served::start_at(&root, &["--qa-actor-hold", &fifo_arg])?,
        fifo,
    ))
}

/// Lets the held runtime go: opening the FIFO for writing completes its open for reading.
fn release(fifo: &Path) -> TestResult {
    drop(std::fs::OpenOptions::new().write(true).open(fifo)?);
    Ok(())
}

fn run_params(op: &str) -> Value {
    json!({"op_id": op, "program": {"argv": ["/bin/sh"]}, "geometry": {"cols": 80, "rows": 24},
        "cwd": "/tmp", "env": {"PATH": "/usr/bin:/bin"}, "name": "held", "labels": null,
        "parent": null, "child_policy": null, "keep": null, "first_message": null,
        "machine": null})
}

/// Asserts `error` is the server's pending answer for `op`.
fn pending(error: &Value, op: &str) {
    assert_eq!(
        error.get("kind"),
        Some(&json!("deadline_exceeded")),
        "{error}"
    );
    assert_eq!(error.get("op_id"), Some(&json!(op)), "{error}");
    assert_eq!(error.get("origin"), Some(&json!("server")), "{error}");
}

#[test]
fn the_client_waits_longer_than_the_server_deadline() {
    let limits = Limits::v1();
    assert!(limits.client_bound_ms().get() > limits.rpc_default_deadline_ms.get());
}

#[test]
fn a_held_terminal_run_answers_pending_with_its_op_id_and_op_get_settles_it() -> TestResult {
    let (served, fifo) = held()?;
    let op = op_id()?;
    let refused = served.refused(
        &[
            "call",
            "terminal.run",
            "--params",
            &run_params(&op).to_string(),
        ],
        13,
    )?;
    pending(&refused, &op);
    release(&fifo)?;
    // The actor runs one job at a time, so op.get is answered after the held run settled.
    let settled = served.ok(&[
        "call",
        "op.get",
        "--params",
        &json!({"op_id": op}).to_string(),
    ])?;
    assert_eq!(settled.get("state"), Some(&json!("committed")), "{settled}");
    served.stop()?;
    Ok(())
}

#[test]
fn a_restart_answered_pending_is_still_handed_over_and_op_get_resolves_it() -> TestResult {
    let (mut served, fifo) = held()?;
    let run = op_id()?;
    // Once terminal.run answered pending, the runtime holds the control on the FIFO.
    let refused = served.refused(
        &[
            "call",
            "terminal.run",
            "--params",
            &run_params(&run).to_string(),
        ],
        13,
    )?;
    pending(&refused, &run);
    let restart = op_id()?;
    let params = json!({"op_id": restart}).to_string();
    let refused = served.refused(&["call", "server.restart", "--params", &params], 13)?;
    pending(&refused, &restart);
    release(&fifo)?;
    let announced = served.announcement()?;
    assert!(
        announced.pointer("/restarted/server_boot_id").is_some(),
        "{announced}"
    );
    let settled = served.ok(&[
        "call",
        "op.get",
        "--params",
        &json!({"op_id": restart}).to_string(),
    ])?;
    assert_eq!(settled.get("state"), Some(&json!("committed")), "{settled}");
    served.stop()?;
    Ok(())
}

#[test]
fn a_qa_seam_is_refused_on_a_root_that_is_not_a_qa_fixture_root() -> TestResult {
    let root = fresh_root()?;
    let output = Command::new(env!("CARGO_BIN_EXE_ovis-terminal"))
        .args(["serve", "--root"])
        .arg(&root)
        .args(["--stdin-lifetime", "--qa-holder-poll-ms", "1"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()?;
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("accept the QA seams"), "{output:?}");
    std::fs::remove_dir_all(root)?;
    Ok(())
}

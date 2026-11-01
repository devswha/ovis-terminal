//! A program the server launches holds none of the server's standard streams: those are
//! the pipes the server's parent reads (its ready line, announcements and report), and a
//! program that outlives the server (programs survive restarts and crashes by design)
//! would keep them open, so the parent would never see end of file. The server's streams
//! are pipes, so the program, whose own streams are its terminal, must hold no FIFO at all;
//! it lists its own descriptors through /dev/fd, which Linux and macOS both provide.

use serde_json::{Value, json};

use super::support::{Served, TestResult, op_id};

/// Prints every descriptor of the shell that is a FIFO, then a marker.
const SCAN: &str =
    r#"for f in /dev/fd/*; do [ -p "$f" ] && echo "fifo:$f"; done; echo scanned-$((1+1))"#;

#[test]
fn a_launched_program_holds_none_of_the_servers_standard_streams() -> TestResult {
    let served = Served::start()?;
    served.run_shell("shell")?;
    let target = json!({"address": "t1"});
    let sent = served.ok(&[
        "call",
        "input.send",
        "--params",
        &json!({"op_id": op_id()?,
        "target": target, "expect": null, "text": SCAN, "submit": "enter",
        "lease_id": null})
        .to_string(),
    ])?;
    let cursor = sent
        .pointer("/receipt/output_cursor/offset")
        .cloned()
        .ok_or("cursor")?;
    let waited = served.ok(&[
        "call",
        "terminal.wait_output",
        "--params",
        &json!({
        "target": target, "cursor": {"offset": cursor}, "pattern": {"text": "scanned-2"},
        "timeout_ms": "10000"})
        .to_string(),
    ])?;
    assert_eq!(waited.get("outcome"), Some(&json!("matched")), "{waited}");
    let screen = served.ok(&[
        "call",
        "terminal.screen",
        "--params",
        &json!({"target": target, "format": "text"}).to_string(),
    ])?;
    let held: Vec<&str> = screen
        .get("lines")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|line| line.starts_with("fifo:"))
        .collect();
    assert!(held.is_empty(), "the program holds FIFOs {held:?}");
    served.stop()?;
    Ok(())
}

//! What the todo-62 attach tests share: the program's incarnation and an attach that has
//! been seen to reach the program.

use std::time::Duration;

use serde_json::{Value, json};

use crate::support::{Outer, Served, TestResult};

/// The launched program's pid and start token, which no attach, takeover or detach may
/// change.
///
/// # Errors
///
/// Fails when the process record lacks either field.
pub fn incarnation(served: &Served) -> TestResult<Value> {
    let info = served.call(
        "terminal.process_info",
        &json!({"target": {"address": "t1"}}),
    )?;
    let field = |path: &str| {
        info.pointer(path)
            .cloned()
            .ok_or_else(|| format!("{path} in {info}"))
    };
    let pid = field("/launcher/pid")?;
    let token = field("/launcher/start_token")?;
    Ok(json!([pid, token]))
}

/// Attaches with `args` and waits until a marker typed through the attach is echoed.
///
/// # Errors
///
/// Fails when a process or PTY step fails.
///
/// # Panics
///
/// Panics when the program never answers through the attach.
pub fn attached(served: &Served, args: &[&str], marker: &str) -> TestResult<Outer> {
    let mut outer = Outer::attach(served, args)?;
    let shown = (0..20).any(|_| {
        outer
            .type_bytes(format!("echo {marker}-$((1+1))\r").as_bytes())
            .is_ok()
            && outer.shows(&format!("{marker}-2"), Duration::from_secs(1))
    });
    assert!(
        shown,
        "{marker}: the program never answered through the attach"
    );
    Ok(outer)
}

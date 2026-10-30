//! Windows CI smoke tier: ConPTY spawn, output before exit, resize, kill,
//! explicit unsupported start tokens and named-pipe first-instance ownership.
//! It is not full Windows QA.
//!
//! A capability the platform does not deliver is a typed finding: one
//! `PROBE finding` record (typed kind, exact OS error, source chain) plus a
//! GitHub `::error::` annotation, blocking the owning backend (Todos 53/54).
//! A finding is never counted as the capability, and by default it does not
//! fail the smoke; `OVIS_PROBE_REQUIRE_PASS=1` turns every finding into a test
//! failure, which is how Todos 53/54 prove the gap closed.

use std::time::Duration;

use ovis_terminal_holder::protocol::{Exit, Message, TerminalTarget};
use ovis_terminal_probes::platform::{
    Grid, ProbeError, PtySession, observe_process, probe_broken_named_pipe, probe_mode_round_trip,
    probe_named_pipe, probe_stalled_writer,
};
use portable_pty::CommandBuilder;
use serde_json::{Value, json};

use crate::{assert_spawn_failure_is_typed, record, terminal, through_holder_frame};

/// Bound on one wait for ConPTY output or exit.
const WAIT: Duration = Duration::from_secs(10);
/// Device status report; ConPTY can hold output until the terminal answers it.
const CURSOR_QUERY: &[u8] = b"\x1b[6n";
const CURSOR_REPLY: &[u8] = b"\x1b[1;1R";

fn cmd(script: &str) -> CommandBuilder {
    let mut command = CommandBuilder::new("cmd.exe");
    command.args(["/d", "/c", script]);
    command
}

fn finding(capability: &str, detail: &Value) {
    record(
        "finding",
        &json!({ "capability": capability, "blocks": "todo-53/54", "detail": detail }),
    );
    println!("::error title=platform probe finding::{capability}: {detail}");
    assert!(
        std::env::var_os("OVIS_PROBE_REQUIRE_PASS").is_none_or(|value| value != "1"),
        "OVIS_PROBE_REQUIRE_PASS=1 and the platform lacks {capability}: {detail}"
    );
}

/// The typed kind, hint, message and source chain (the exact OS error).
fn error_detail(error: &ProbeError) -> Value {
    let mut chain = Vec::new();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        chain.push(cause.to_string());
        source = cause.source();
    }
    json!({ "kind": error.kind(), "hint": error.hint(), "error": error.to_string(), "source": chain })
}

/// Keeps a measured value, or records the typed failure as a finding.
fn measured<T>(capability: &str, outcome: Result<T, ProbeError>) -> Option<T> {
    outcome
        .map_err(|error| finding(capability, &error_detail(&error)))
        .ok()
}

/// Waits for `marker`. When it does not arrive, the typed timeout and the
/// exact bytes that did arrive become a finding; if those bytes contain the
/// cursor query, the probe answers it once and records whether output flows.
/// Returns whether the marker arrived.
fn await_marker(session: &mut PtySession, marker: &[u8], capability: &str) -> bool {
    let Err(error) = session.read_until(marker, WAIT) else {
        return true;
    };
    let unconsumed = session.unconsumed().to_vec();
    let cursor_query_seen = unconsumed
        .windows(CURSOR_QUERY.len())
        .any(|window| window == CURSOR_QUERY);
    let retried = cursor_query_seen.then(|| {
        session
            .write(CURSOR_REPLY)
            .and_then(|()| session.read_until(marker, WAIT))
            .map(drop)
    });
    let after_cursor_reply = retried.as_ref().map(|outcome| match outcome {
        Ok(()) => "marker_arrived",
        Err(error) => error.kind(),
    });
    finding(
        capability,
        &json!({
            "error": error_detail(&error),
            "waited_ms": WAIT.as_millis().to_string(),
            "received_bytes": session.received(),
            "unconsumed": unconsumed.escape_ascii().to_string(),
            "cursor_query_seen": cursor_query_seen,
            "after_cursor_reply": after_cursor_reply,
        }),
    );
    retried.is_some_and(|outcome| outcome.is_ok())
}

fn output_precedes_exit() {
    let Some(mut session) = measured(
        "conpty_spawn",
        PtySession::spawn(cmd("echo OVIS-FINAL-BYTES& exit 7"), Grid::new(80, 24)),
    ) else {
        return;
    };
    let pid = session.pid();
    let token = pid.map(observe_process);
    record(
        "start_token",
        &token.map(|outcome| outcome.map_err(|error| error_detail(&error))),
    );
    if !await_marker(
        &mut session,
        b"OVIS-FINAL-BYTES",
        "conpty_output_before_exit",
    ) {
        record(
            "conpty_exit_without_output",
            &session
                .wait_exit(WAIT)
                .map_err(|error| error_detail(&error)),
        );
        return;
    }
    let Some(exit) = measured("conpty_exit_status", session.wait_exit(WAIT)) else {
        return;
    };
    if exit.code != 7 {
        finding(
            "conpty_exit_code",
            &json!({ "expected": 7, "observed": exit }),
        );
    }
    let (terminal_id, terminal_incarnation, process_incarnation) = terminal("probe-conpty");
    let target = TerminalTarget {
        terminal_id,
        terminal_incarnation,
        process_incarnation: Some(process_incarnation),
    };
    let exited = Message::Exit(Exit {
        target,
        code: i32::try_from(exit.code).ok(),
        signal: None,
        final_offset: session.received().get(),
    });
    assert_eq!(through_holder_frame(&exited), exited);
    record(
        "conpty_exit",
        &json!({ "pid": pid, "exit": exit, "holder_frame": exited }),
    );
    let drained = session.drain_to_end(Duration::from_secs(2));
    record(
        "conpty_eof_after_exit",
        &drained
            .map(|(_tail, end)| end)
            .map_err(|error| error_detail(&error)),
    );
}

fn resize_and_kill() {
    let Some(mut session) = measured(
        "conpty_spawn",
        PtySession::spawn(cmd("ping -n 30 127.0.0.1 >NUL"), Grid::new(80, 24)),
    ) else {
        return;
    };
    let requested = Grid::new(132, 40);
    if measured("conpty_resize", session.resize(requested)).is_some() {
        match measured("conpty_read_size", session.grid()) {
            Some(grid) if grid == requested => record("conpty_resize", &grid),
            Some(grid) => finding(
                "conpty_resize",
                &json!({ "requested": requested, "observed": grid }),
            ),
            None => {}
        }
    }
    if measured("conpty_backend_kill", session.kill()).is_some() {
        record(
            "conpty_kill_exit",
            &session
                .wait_exit(WAIT)
                .map_err(|error| error_detail(&error)),
        );
    }
}

/// Runs the Windows smoke happy path.
///
/// # Panics
///
/// When a holder frame does not round-trip, or under
/// `OVIS_PROBE_REQUIRE_PASS=1` when any capability is a finding.
pub fn happy() {
    output_precedes_exit();
    resize_and_kill();
    let name = format!("ovis-probe-{}", std::process::id());
    if let Some(pipe) = measured("named_pipe", probe_named_pipe(&name)) {
        if !pipe.first_instance_exclusive || pipe.exchanged.get() == 0 {
            finding("named_pipe_first_instance", &json!(pipe));
        }
        record("named_pipe", &pipe);
    }
    if let Some(modes) = measured(
        "conpty_mode_round_trip",
        probe_mode_round_trip(Grid::new(80, 24), WAIT),
    ) {
        record("conpty_modes_observed", &modes);
    }
}

/// Runs the Windows smoke injected failures.
///
/// # Panics
///
/// When a spawn or pipe failure is not rejected with the asserted typed kind,
/// or under `OVIS_PROBE_REQUIRE_PASS=1` when any capability is a finding.
pub fn failure() {
    if let Some(stall) = measured("conpty_stall", probe_stalled_writer(WAIT)) {
        if !stall.backend_kill_ended_program {
            finding("conpty_stall_backend_kill", &json!(stall));
        }
        record("stall", &stall);
    }
    let error = PtySession::spawn(
        CommandBuilder::new(r"C:\nonexistent\ovis-probe.exe"),
        Grid::new(80, 24),
    )
    .unwrap_err();
    assert_spawn_failure_is_typed(&error);
    let missing = probe_broken_named_pipe(&format!("ovis-probe-missing-{}", std::process::id()))
        .unwrap()
        .unwrap_err();
    assert_eq!(missing.kind(), "endpoint_failed");
    record("missing_pipe", &error_detail(&missing));
}

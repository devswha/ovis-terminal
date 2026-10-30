//! Windows CI smoke tier: ConPTY spawn, output before exit, resize, explicit
//! unsupported start tokens and named-pipe first-instance ownership. It is not
//! full Windows QA.
//!
//! A capability the platform does not deliver is reported as a typed finding:
//! one `PROBE finding` record plus a GitHub `::error::` annotation, blocking the
//! owning backend (Todos 53/54). It is never counted as the capability.

use std::time::Duration;

use ovis_terminal_holder::protocol::{Exit, Message, TerminalTarget};
use ovis_terminal_probes::platform::{
    Grid, PtySession, observe_process, probe_broken_named_pipe, probe_mode_round_trip,
    probe_named_pipe, probe_stalled_writer,
};
use portable_pty::CommandBuilder;
use serde_json::json;

use crate::{BOUND, assert_spawn_failure_is_typed, record, terminal, through_holder_frame};

/// Bound on one wait for ConPTY output.
const WAIT: Duration = Duration::from_secs(10);
/// Device status report; ConPTY can hold output until the terminal answers it.
const CURSOR_QUERY: &[u8] = b"\x1b[6n";
const CURSOR_REPLY: &[u8] = b"\x1b[1;1R";

fn cmd(script: &str) -> CommandBuilder {
    let mut command = CommandBuilder::new("cmd.exe");
    command.args(["/d", "/c", script]);
    command
}

fn finding(capability: &str, detail: &serde_json::Value) {
    record(
        "finding",
        &json!({ "capability": capability, "blocks": "todo-53/54", "detail": detail }),
    );
    println!("::error title=platform probe finding::{capability}: {detail}");
}

/// Waits for `marker`. When it does not arrive, the typed timeout and the
/// exact bytes that did arrive become a finding; if those bytes contain the
/// cursor query, the probe answers it once and records whether output flows.
/// Returns whether the marker arrived.
fn await_marker(session: &mut PtySession, marker: &[u8], capability: &str) -> bool {
    let Err(error) = session.read_until(marker, WAIT) else {
        return true;
    };
    assert_eq!(error.kind(), "timeout", "{error:?}");
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
            "error_kind": error.kind(),
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
    let mut session =
        PtySession::spawn(cmd("echo OVIS-FINAL-BYTES& exit 7"), Grid::new(80, 24)).unwrap();
    let pid = session.pid().unwrap();
    let unsupported = observe_process(pid).unwrap_err();
    assert_eq!(unsupported.kind(), "unsupported");
    record(
        "start_token",
        &json!({ "kind": unsupported.kind(), "hint": unsupported.hint() }),
    );
    if !await_marker(
        &mut session,
        b"OVIS-FINAL-BYTES",
        "conpty_output_before_exit",
    ) {
        let exit = session.wait_exit(WAIT).map_err(|error| error.kind());
        record(
            "conpty_exit_without_output",
            &json!({ "pid": pid, "exit": exit }),
        );
        return;
    }
    let exit = session.wait_exit(BOUND).unwrap();
    assert_eq!(exit.code, 7);
    let (terminal_id, terminal_incarnation, process_incarnation) = terminal("probe-conpty");
    let target = TerminalTarget {
        terminal_id,
        terminal_incarnation,
        process_incarnation: Some(process_incarnation),
    };
    let final_offset = session.received().get();
    let exited = Message::Exit(Exit {
        target,
        code: Some(7),
        signal: None,
        final_offset,
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
            .map_err(|error| error.kind()),
    );
}

fn resize_is_accepted() {
    let mut session =
        PtySession::spawn(cmd("ping -n 30 127.0.0.1 >NUL"), Grid::new(80, 24)).unwrap();
    session.resize(Grid::new(132, 40)).unwrap();
    assert_eq!(session.grid().unwrap(), Grid::new(132, 40));
    session.kill().unwrap();
    record(
        "conpty_resize",
        &json!({ "grid": session.grid().unwrap(), "exit": session.wait_exit(BOUND).unwrap() }),
    );
}

/// Runs the Windows smoke happy path.
///
/// # Panics
///
/// When the host behaves other than the smoke asserts.
pub fn happy() {
    output_precedes_exit();
    resize_is_accepted();
    let name = format!("ovis-probe-{}", std::process::id());
    let pipe = probe_named_pipe(&name).unwrap();
    assert!(pipe.first_instance_exclusive, "{pipe:?}");
    assert!(pipe.exchanged.get() > 0);
    record("named_pipe", &pipe);
    match probe_mode_round_trip(Grid::new(80, 24), WAIT) {
        Ok(modes) => record("conpty_modes_observed", &modes),
        Err(error) => finding(
            "conpty_mode_round_trip",
            &json!({ "error_kind": error.kind(), "hint": error.hint() }),
        ),
    }
}

/// Runs the Windows smoke injected failures.
///
/// # Panics
///
/// When a failure is not rejected with the asserted typed kind.
pub fn failure() {
    match probe_stalled_writer(WAIT) {
        Ok(stall) => {
            assert!(stall.backend_kill_ended_program, "{stall:?}");
            record("stall", &stall);
        }
        Err(error) => {
            assert_eq!(error.kind(), "timeout", "{error:?}");
            finding(
                "conpty_stall_probe_output",
                &json!({ "error_kind": error.kind(), "hint": error.hint() }),
            );
        }
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
    record("missing_pipe", &json!({ "kind": missing.kind() }));
}

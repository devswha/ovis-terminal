//! Windows CI smoke tier: ConPTY spawn, output before exit, resize, explicit
//! unsupported start tokens and named-pipe first-instance ownership. It is not
//! full Windows QA.

use ovis_terminal_holder::protocol::{Exit, Message, TerminalTarget};
use ovis_terminal_probes::platform::{
    Grid, PtySession, observe_process, probe_broken_named_pipe, probe_mode_round_trip,
    probe_named_pipe, probe_stalled_writer,
};
use portable_pty::CommandBuilder;

use crate::{BOUND, assert_spawn_failure_is_typed, record, terminal, through_holder_frame};

fn cmd(script: &str) -> CommandBuilder {
    let mut command = CommandBuilder::new("cmd.exe");
    command.args(["/d", "/c", script]);
    command
}

fn output_precedes_exit() {
    let mut session =
        PtySession::spawn(cmd("echo OVIS-FINAL-BYTES& exit 7"), Grid::new(80, 24)).unwrap();
    let pid = session.pid().unwrap();
    session.read_until(b"OVIS-FINAL-BYTES", BOUND).unwrap();
    let exit = session.wait_exit(BOUND).unwrap();
    assert_eq!(exit.code, 7);
    let unsupported = observe_process(pid).unwrap_err();
    assert_eq!(unsupported.kind(), "unsupported");
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
        &serde_json::json!({ "pid": pid, "exit": exit, "holder_frame": exited }),
    );
    record(
        "start_token",
        &serde_json::json!({ "kind": unsupported.kind(), "hint": unsupported.hint() }),
    );
    let drained = session.drain_to_end(std::time::Duration::from_secs(2));
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
        &serde_json::json!({ "grid": session.grid().unwrap(), "exit": session.wait_exit(BOUND).unwrap() }),
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
    let modes = probe_mode_round_trip(Grid::new(80, 24), BOUND);
    record(
        "conpty_modes_observed",
        &modes.map_err(|error| error.kind()),
    );
}

/// Runs the Windows smoke injected failures.
///
/// # Panics
///
/// When a failure is not rejected with the asserted typed kind.
pub fn failure() {
    let stall = probe_stalled_writer(BOUND).unwrap();
    assert!(stall.backend_kill_ended_program, "{stall:?}");
    record("stall", &stall);
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
    record(
        "missing_pipe",
        &serde_json::json!({ "kind": missing.kind() }),
    );
}

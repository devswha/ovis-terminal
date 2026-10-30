//! Linux/macOS injected failures: a program that never reads and ignores
//! SIGHUP, a missing executable, exhausted descriptors, a missing working
//! directory and a broken socket endpoint.

use std::io;
use std::process::{Command, Stdio};

use ovis_terminal_holder::protocol::{FrameDecoder, Message, SpawnFailureKind};
use ovis_terminal_probes::platform::{
    Grid, PtySession, probe_broken_unix_endpoint, probe_stalled_writer,
};
use portable_pty::CommandBuilder;

use crate::{BOUND, ScratchDir, assert_spawn_failure_is_typed, record};

const MISSING_DIR: &str = "/nonexistent/ovis-probe-cwd";

fn unread_input_stays_killable() {
    let stall = probe_stalled_writer(BOUND).unwrap();
    assert!(
        !stall.backend_kill_ended_program,
        "SIGHUP ended a program that ignores it: {stall:?}"
    );
    assert!(stall.exit.signal.is_some(), "{stall:?}");
    assert!(stall.kill_to_exit.as_duration() < BOUND, "{stall:?}");
    record("stall", &stall);
}

fn missing_executable_is_spawn_failed() {
    let error = PtySession::spawn(
        CommandBuilder::new("/nonexistent/ovis-probe-program"),
        Grid::new(80, 24),
    )
    .unwrap_err();
    assert_spawn_failure_is_typed(&error);
}

/// Under `ulimit -n 4` the fixture can start but not open a pseudo-terminal
/// pair; it must answer with the holder's `SpawnFailed { kind: unavailable }`.
fn exhausted_descriptors_are_unavailable() {
    let fixture = env!("CARGO_BIN_EXE_ovis-terminal-probe-holder");
    let output = Command::new("sh")
        .args(["-c", r#"ulimit -n 4 && exec "$0" openpty"#, fixture])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{}: {stderr}", output.status);
    let messages = FrameDecoder::default().push(&output.stdout).unwrap();
    let [Message::SpawnFailed(failed)] = messages.as_slice() else {
        panic!("expected one SpawnFailed frame, got {messages:?}");
    };
    assert_eq!(failed.kind, SpawnFailureKind::Unavailable);
    record(
        "openpty_refused",
        &serde_json::json!({ "frame": failed, "backend": stderr.trim() }),
    );
}

/// portable-pty 0.9.0 drops a working directory that is not a directory and
/// starts the program in `$HOME`; the program must never report the missing
/// directory as its own.
fn missing_working_directory_is_not_reported_as_used() {
    let mut command = CommandBuilder::new("sh");
    command.cwd(MISSING_DIR);
    command.env("PS1", "");
    match PtySession::spawn(command, Grid::new(80, 24)) {
        Err(error) => assert_eq!(error.kind(), "spawn_failed", "{error:?}"),
        Ok(mut session) => {
            session
                .write(b"printf 'C''WD %s\\n' \"$(pwd)\"; exit\n")
                .unwrap();
            session.read_until(b"CWD ", BOUND).unwrap();
            let used = String::from_utf8(session.read_until(b"\n", BOUND).unwrap()).unwrap();
            assert_ne!(used.trim(), MISSING_DIR);
            session.finish(BOUND).unwrap();
            record(
                "missing_cwd",
                &serde_json::json!({ "requested": MISSING_DIR, "used": used.trim() }),
            );
        }
    }
}

fn broken_endpoint_is_typed() {
    let scratch = ScratchDir::new("broken");
    let broken = probe_broken_unix_endpoint(&scratch.0).unwrap();
    let connect = broken.connect_to_removed.unwrap_err();
    let write = broken.write_to_closed_peer.unwrap_err();
    let io_kind = |error: &ovis_terminal_probes::platform::ProbeError| {
        std::error::Error::source(error)
            .and_then(|source| source.downcast_ref::<io::Error>())
            .map(io::Error::kind)
            .unwrap()
    };
    assert_eq!(
        (connect.kind(), write.kind()),
        ("endpoint_failed", "endpoint_failed")
    );
    assert_eq!(io_kind(&connect), io::ErrorKind::NotFound);
    assert!(matches!(
        io_kind(&write),
        io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
    ));
    record(
        "broken_endpoint",
        &serde_json::json!({
            "connect_to_removed": io_kind(&connect).to_string(),
            "write_to_closed_peer": io_kind(&write).to_string(),
        }),
    );
}

/// Runs every Unix injected-failure probe.
///
/// # Panics
///
/// When a failure is not rejected with the asserted typed kind.
pub fn run() {
    unread_input_stays_killable();
    missing_executable_is_spawn_failed();
    exhausted_descriptors_are_unavailable();
    missing_working_directory_is_not_reported_as_used();
    broken_endpoint_is_typed();
}

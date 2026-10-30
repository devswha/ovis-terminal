//! Linux/macOS injected failures: a program that never reads and ignores
//! SIGHUP, a missing executable, exhausted descriptors, a missing working
//! directory and a broken socket endpoint.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use std::{io, thread};

use ovis_terminal_holder::protocol::{FrameDecoder, Message, SpawnFailureKind};
use ovis_terminal_probes::platform::{
    Grid, PtySession, observe_process, probe_broken_unix_endpoint, probe_stalled_writer,
};
use portable_pty::CommandBuilder;

use crate::{BOUND, ScratchDir, assert_spawn_failure_is_typed, record};

const STALL_THREADS: &str = "probe-stall";
const MISSING_DIR: &str = "/nonexistent/ovis-probe-cwd";

/// How long a joined thread may linger in `/proc/self/task`: `pthread_join`
/// returns once the kernel clears the child's tid, slightly before the kernel
/// removes the task entry.
const TEARDOWN_DEADLINE: Duration = Duration::from_secs(2);

/// What waiting for a thread family to disappear found. `Unavailable` is a
/// third state, never folded into `Gone`: where the OS offers no thread
/// listing the probe cannot know.
#[derive(Debug, PartialEq, Eq)]
enum Teardown {
    Gone,
    Leftover(Vec<String>),
    Unavailable { reason: io::ErrorKind },
}

/// Names of this test process's threads that start with `prefix`; on Linux the
/// kernel exposes each thread's name under `/proc/self/task`. The error is the
/// kind of the failed listing (`NotFound` where `/proc` does not exist).
fn thread_names(prefix: &str) -> Result<Vec<String>, io::ErrorKind> {
    let tasks = std::fs::read_dir("/proc/self/task").map_err(|error| error.kind())?;
    Ok(tasks
        .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
        .map(|name| name.trim_end().to_owned())
        .filter(|name| name.starts_with(prefix))
        .collect())
}

/// Waits until at most `baseline` threads named `prefix…` remain; a thread
/// that never exits is reported by name once `deadline` passes.
fn await_threads_at_most(prefix: &str, baseline: usize, deadline: Duration) -> Teardown {
    let started = Instant::now();
    loop {
        match thread_names(prefix) {
            Err(reason) => return Teardown::Unavailable { reason },
            Ok(names) if names.len() <= baseline => return Teardown::Gone,
            Ok(names) if started.elapsed() >= deadline => return Teardown::Leftover(names),
            Ok(_) => thread::yield_now(),
        }
    }
}

/// The outcome this OS must produce when `linux` is what Linux produces.
fn on_this_os(linux: Teardown) -> Teardown {
    if cfg!(target_os = "linux") {
        linux
    } else {
        Teardown::Unavailable {
            reason: io::ErrorKind::NotFound,
        }
    }
}

/// The blocked writer runs in a fixture subprocess; the probe must return with
/// no thread, fixture or program left behind.
fn unread_input_stays_killable() {
    let threads_before = thread_names(STALL_THREADS).map_or(0, |names| names.len());
    let fixture = Path::new(env!("CARGO_BIN_EXE_ovis-terminal-probe-stall"));
    let stall = probe_stalled_writer(fixture, BOUND).unwrap();
    assert_eq!(
        await_threads_at_most(STALL_THREADS, threads_before, TEARDOWN_DEADLINE),
        on_this_os(Teardown::Gone),
        "the probe left a thread behind"
    );
    for pid in [stall.fixture, stall.program] {
        assert_eq!(
            observe_process(pid).unwrap_err().kind(),
            "process_record_unreadable",
            "{pid} outlived the probe"
        );
    }
    assert!(
        !stall.report.backend_kill_ended_program,
        "SIGHUP ended a program that ignores it: {stall:?}"
    );
    assert!(stall.report.exit.signal.is_some(), "{stall:?}");
    assert!(stall.report.kill_to_exit.as_duration() < BOUND, "{stall:?}");
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

#[test]
fn an_exited_thread_is_reported_gone() {
    let finished = thread::Builder::new()
        .name("t72-exits".into())
        .spawn(|| ())
        .unwrap();
    finished.join().unwrap();
    assert_eq!(
        await_threads_at_most("t72-exits", 0, TEARDOWN_DEADLINE),
        on_this_os(Teardown::Gone)
    );
}

#[test]
fn a_thread_that_never_exits_is_reported_at_the_deadline() {
    let (release, parked) = mpsc::channel::<()>();
    let (running, started_up) = mpsc::channel::<()>();
    let stuck = thread::Builder::new()
        .name("t72-parked".into())
        .spawn(move || {
            running.send(()).unwrap();
            parked.recv()
        })
        .unwrap();
    started_up.recv().unwrap();
    let started = Instant::now();
    let left = await_threads_at_most("t72-parked", 0, Duration::from_millis(200));
    let leftover = Teardown::Leftover(vec!["t72-parked".to_owned()]);
    let waited_out = left == leftover;
    assert_eq!(left, on_this_os(leftover));
    assert!(!waited_out || started.elapsed() >= Duration::from_millis(200));
    release.send(()).unwrap();
    stuck.join().unwrap().unwrap();
    assert_eq!(
        await_threads_at_most("t72-parked", 0, TEARDOWN_DEADLINE),
        on_this_os(Teardown::Gone)
    );
}

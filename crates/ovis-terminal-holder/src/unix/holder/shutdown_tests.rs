//! Shutdown against real process evidence before output completion.

use std::ffi::OsString;
use std::sync::{Arc, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};

use super::Holder;
use super::ending::{Completion, ExitEvidence};
use super::state::{LiveTerminal, Terminal};
use super::threads::{Waiter, wait_for_exit};
use crate::process::{ProcessId, SignalScope, send_signal};
use crate::protocol::{
    HOLDER_PROTOCOL_GENERATION, HeldTerminal, Hello, Message, PreviousCoreState,
    ProcessIncarnation, SignalKind,
};
use crate::unix::config::{ProcessEnvironment, QueueLimits};
use crate::unix::drain::DrainSignal;
use crate::unix::input::InputQueue;
use crate::unix::launch::{Grid, LaunchSpec, launch};
use crate::unix::tail::RetainedTail;
use crate::unix::test_fixtures::{WAIT, config, shell};

const SHUTDOWN_BUDGET: Duration = Duration::from_secs(2);

fn retained_child() -> (Holder, Waiter) {
    let holder = Holder::new(config());
    let request = shell("t-reaped", "exit 0");
    let spec = LaunchSpec::new(
        request.argv.into_iter().map(OsString::from).collect(),
        ProcessEnvironment::new(
            request.cwd,
            request
                .env
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        ),
        Grid::new(80, 24).expect("grid"),
    );
    let process = launch(&spec).expect("real PTY child");
    let terminal_id = request.terminal_id;
    holder.shared.lock().terminals.insert(
        terminal_id.clone(),
        Terminal {
            held: HeldTerminal {
                terminal_id: terminal_id.clone(),
                terminal_incarnation: request.terminal_incarnation,
                process_incarnation: Some(ProcessIncarnation::new(1)),
                pid: Some(process.pid.get()),
                start_token: Some(process.start_token),
                output_offset: 0,
                input_sequence: 0,
                exit: None,
            },
            tail: RetainedTail::new(16),
            live: Some(LiveTerminal {
                master: process.master,
                input: Arc::new(InputQueue::new(QueueLimits::new(16, 2))),
                pid: process.pid,
                pump_stop: Some(process.pump_stop),
            }),
            geometry_epoch: None,
            admitted_sequence: None,
            io_errors: Vec::new(),
            first_io_failure: None,
            completion: Completion::Running,
            waiter: super::state::WaiterState::Waiting,
        },
    );
    let drain = Arc::new(DrainSignal::default());
    drain.finish();
    let waiter = Waiter {
        shared: Arc::clone(&holder.shared),
        terminal_id,
        child: process.child,
        slave: process.slave,
        probe: process.probe,
        drain,
    };
    (holder, waiter)
}

pub(super) fn wait_until_reaped(holder: &Holder, terminal_id: &crate::protocol::TerminalId) {
    let (state, wait) = holder
        .shared
        .reaped
        .wait_timeout_while(holder.shared.lock(), WAIT, |state| {
            state.terminals.get(terminal_id).is_some_and(|terminal| {
                !matches!(terminal.waiter, super::state::WaiterState::Reaped)
            })
        })
        .unwrap_or_else(PoisonError::into_inner);
    assert!(!wait.timed_out(), "the waiter did not reap the leader");
    assert!(state.terminals.contains_key(terminal_id));
    drop(state);
}

fn holder_with_orphan(terminal_id: &str, script: &str) -> (Holder, crate::protocol::Spawned) {
    let holder = Holder::new(config());
    let hello = Hello {
        generation: HOLDER_PROTOCOL_GENERATION,
        core_build: crate::protocol::CoreBuildId::new("round-2-test".to_owned()),
        previous: PreviousCoreState::Crash,
    };
    let link = holder.adopt(&hello).expect("adopt");
    assert!(matches!(
        link.recv_timeout(WAIT).expect("welcome"),
        Message::Welcome(_)
    ));
    let spawned = holder
        .spawn_wire(&shell(terminal_id, script))
        .expect("spawn");
    let mut seen = Vec::new();
    while !seen.windows(16).any(|bytes| bytes == b"descendant-ready") {
        match link.recv_timeout(WAIT).expect("descendant marker") {
            Message::Output(output) => seen.extend(output.bytes),
            Message::Spawned(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    wait_until_reaped(&holder, &spawned.terminal_id);
    (holder, spawned)
}

#[test]
fn shutdown_reports_already_exited_when_waiter_reaped_before_output_completion() {
    // Given: the real waiter reaps a real PTY child, with no output pump yet.
    let (holder, waiter) = retained_child();
    let terminal_id = waiter.terminal_id.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    wait_for_exit(waiter, &sender);
    let evidence = receiver
        .recv_timeout(WAIT)
        .expect("waiter completed")
        .evidence;
    assert!(matches!(
        evidence,
        ExitEvidence::Status {
            code: Some(0),
            signal: None
        }
    ));
    {
        let state = holder.shared.lock();
        let terminal = state.terminals.get(&terminal_id).expect("retained");
        assert!(terminal.live.is_some());
        assert!(terminal.held.exit.is_none());
        drop(state);
    }

    // When: shutdown runs before output completion can remove the live PTY.
    let report = holder.shutdown().expect("already exited is not a failure");

    // Then: it reports the evidenced exit instead of attempting delivery.
    assert_eq!(report.already_exited, 1);
}

#[test]
fn shutdown_reports_zombie_without_signalling_before_waiter_reaps() {
    // Given: the kernel reports exit, but NOWAIT keeps the child unreaped.
    let (holder, waiter) = retained_child();
    waitid(
        WaitId::Pid(Pid::from_child(&waiter.child)),
        WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
    )
    .expect("real child became waitable");

    // When: shutdown wins the state mutex before the waiter can reap.
    let result = holder.shutdown();
    let (sender, receiver) = mpsc::sync_channel(1);
    wait_for_exit(waiter, &sender);
    receiver.recv_timeout(WAIT).expect("child reaped");

    // Then: the zombie was an already-exited target, not a delivery failure.
    assert_eq!(result.expect("shutdown").already_exited, 1);
}

#[test]
fn shutdown_kills_same_group_descendant_after_leader_is_reaped() {
    // Given: the leader has exited and been reaped, while a same-group child
    // ignores HUP and keeps the PTY slave open.
    let (holder, spawned) = holder_with_orphan(
        "t-orphan-group",
        "trap '' HUP; sleep 777 & echo descendant-ready; exit 0",
    );

    // When: shutdown runs after the leader reap.
    let (sender, receiver) = mpsc::sync_channel(1);
    let shutting_down = holder;
    let shutdown = thread::spawn(move || {
        sender
            .send(shutting_down.shutdown())
            .expect("test still receives shutdown");
    });
    let report = match receiver.recv_timeout(SHUTDOWN_BUDGET) {
        Ok(result) => result.expect("shutdown"),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let cleanup = send_signal(
                SignalScope::ProcessGroup(ProcessId::new(spawned.pid)),
                SignalKind::Kill,
            );
            let cleaned = receiver.recv_timeout(WAIT);
            shutdown.join().expect("shutdown thread cleanup");
            panic!(
                "shutdown did not return within {SHUTDOWN_BUDGET:?}: cleanup={cleanup:?}, result={cleaned:?}"
            );
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            shutdown.join().expect("shutdown thread");
            panic!("shutdown result channel disconnected");
        }
    };
    shutdown.join().expect("shutdown thread");

    // Then: the descendant's PTY closes, every worker joins, and the leader's
    // already-exited outcome remains visible.
    assert_eq!(report.already_exited, 1);
    assert!(
        report
            .terminals
            .get(&spawned.terminal_id)
            .expect("terminal report")
            .all_joined()
    );
}

#[test]
fn shutdown_stops_pump_when_reaped_leader_group_is_gone() {
    // Given: job control moved the PTY-holding child to another group, leaving
    // the reaped leader's group absent.
    let (holder, spawned) = holder_with_orphan(
        "t-orphan-other-group",
        "set -m; trap '' HUP; read gate & echo descendant-ready; exit 0",
    );
    let raw = i32::try_from(spawned.pid).expect("positive pid");
    // SAFETY: kill with signal 0 performs an existence check and touches no memory.
    let status = unsafe { libc::kill(raw.saturating_neg(), 0) };
    assert_eq!(status, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );

    // When: shutdown finds no original process group.
    let started = Instant::now();
    let report = holder.shutdown().expect("shutdown");

    // Then: closing the pump stop endpoint prevents an unbounded worker join.
    assert!(started.elapsed() < SHUTDOWN_BUDGET);
    assert_eq!(report.already_exited, 1);
    assert!(
        report
            .terminals
            .get(&spawned.terminal_id)
            .expect("terminal report")
            .all_joined()
    );
}

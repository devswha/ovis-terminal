//! Shutdown against real process evidence before output completion.

use std::ffi::OsString;
use std::sync::{Arc, mpsc};

use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};

use super::Holder;
use super::ending::{Completion, ExitEvidence};
use super::state::{LiveTerminal, Terminal};
use super::threads::{Waiter, wait_for_exit};
use crate::protocol::{HeldTerminal, ProcessIncarnation};
use crate::unix::config::{ProcessEnvironment, QueueLimits};
use crate::unix::drain::DrainSignal;
use crate::unix::input::InputQueue;
use crate::unix::launch::{Grid, LaunchSpec, launch};
use crate::unix::tail::RetainedTail;
use crate::unix::test_fixtures::{WAIT, config, shell};

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

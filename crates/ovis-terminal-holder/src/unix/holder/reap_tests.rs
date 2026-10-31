//! Hangup is attributed from retained incarnation evidence, never from a reap barrier.
//!
//! A same-group descendant keeps the PTY slave open after the leader exits, so
//! the waiter reaps before the descendant is gone. Hangup must still name the
//! spawned incarnation and still precede a final Exit.

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::Holder;
use super::shutdown_tests::wait_until_reaped;
use crate::process::{ProcessId, SignalScope, send_signal};
use crate::protocol::{
    Exit, HOLDER_PROTOCOL_GENERATION, Hangup, Message, Signal, SignalKind, Spawned,
};
use crate::unix::test_fixtures::{WAIT, config, hello_record, shell, target_of};
use crate::unix::{CoreLink, HolderError, RecvError};

const SHUTDOWN_BUDGET: Duration = Duration::from_secs(2);
const LEADER_EXIT: &str =
    "trap '' HUP; sleep 777 3>\"$OVIS_FIFO\" & printf descendant-ready; exit 3";

/// When the leader's exit ends PTY output depends on the platform.
///
/// Linux keeps the master open until the last slave descriptor closes, so with
/// a descendant holding the slave nothing is published until it dies, and its
/// group stays held (and addressable) by the holder until then. Darwin reports
/// EOF as soon as the session leader exits: the pump publishes Hangup and then
/// Exit while the descendant still lives, and the holder releases the terminal,
/// so its group is no longer addressable through the holder.
const LEADER_EXIT_ENDS_OUTPUT: bool = cfg!(target_os = "macos");

/// The descendant holds the only write end of a FIFO; EOF on the read end means
/// every descriptor it had is closed, so it has exited.
struct Descendant {
    fifo: File,
    path: PathBuf,
}

impl Descendant {
    fn wait_exited(self) {
        let Self { mut fifo, path } = self;
        let (done, exited) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut rest = Vec::new();
            done.send(fifo.read_to_end(&mut rest).map(|_| ()))
                .expect("test still waits");
        });
        exited
            .recv_timeout(WAIT)
            .expect("the descendant exited")
            .expect("FIFO read");
        std::fs::remove_file(path).expect("remove FIFO");
    }
}

/// Spawns `LEADER_EXIT` over an adopted link and returns once its leader is reaped.
fn reaped_leader(terminal_id: &str) -> (Holder, CoreLink, Spawned, Descendant) {
    let path = std::env::temp_dir().join(format!("ovis-t16-{}-{terminal_id}", std::process::id()));
    let made = std::process::Command::new("mkfifo").arg(&path).status();
    assert!(made.expect("mkfifo").success());
    let holder = Holder::new(config());
    let link = holder
        .adopt(&hello_record(HOLDER_PROTOCOL_GENERATION))
        .expect("adopt");
    assert!(matches!(link.recv_timeout(WAIT), Ok(Message::Welcome(_))));
    let mut request = shell(terminal_id, LEADER_EXIT);
    let fifo_path = path.to_str().expect("UTF-8 temp path").to_owned();
    request.env.insert("OVIS_FIFO".to_owned(), fifo_path);
    let spawned = holder.spawn_wire(&request).expect("spawn");
    // Opening the read end blocks until the descendant has opened the write end.
    let fifo = File::open(&path).expect("open FIFO");
    let mut seen = Vec::new();
    while !seen.windows(16).any(|bytes| bytes == b"descendant-ready") {
        match link.recv_timeout(WAIT).expect("descendant marker") {
            Message::Output(output) => seen.extend(output.bytes),
            Message::Spawned(_) => {}
            other => panic!("unexpected {:?}", other.kind()),
        }
    }
    wait_until_reaped(&holder, &spawned.terminal_id);
    (holder, link, spawned, Descendant { fifo, path })
}

/// Messages already queued on `link`, without waiting for more.
fn queued(link: &CoreLink) -> Vec<Message> {
    std::iter::from_fn(|| link.recv_timeout(Duration::ZERO).ok()).collect()
}

/// Receives until the terminal's Exit, appending to `messages`.
fn receive_through_exit(link: &CoreLink, messages: &mut Vec<Message>) {
    while !messages
        .iter()
        .any(|message| matches!(message, Message::Exit(_)))
    {
        messages.push(link.recv_timeout(WAIT).expect("lifecycle event"));
    }
}

/// Kills the released group directly; the holder no longer owns it after Exit.
fn kill_released_group(spawned: &Spawned) {
    send_signal(
        SignalScope::ProcessGroup(ProcessId::new(spawned.pid)),
        SignalKind::Kill,
    )
    .expect("the descendant's group exists");
}

/// Hang-up evidence (with its offset) and the exit, in delivery order.
fn lifecycle(messages: Vec<Message>) -> Vec<Result<(Hangup, u64), Exit>> {
    messages
        .into_iter()
        .filter_map(|message| match message {
            Message::Output(output) => output.hangup.map(|hangup| Ok((hangup, output.offset))),
            Message::Exit(exit) => Some(Err(exit)),
            _ => None,
        })
        .collect()
}

#[test]
fn hangup_after_reap_carries_spawned_incarnation_and_precedes_exit() {
    // Given: the leader is reaped while a descendant still holds the PTY slave.
    let (holder, link, spawned, descendant) = reaped_leader("t-reap-first");
    let mut messages = queued(&link);

    // When: Linux needs the descendant killed before EOF; Darwin ends output at
    // the leader's exit, and the test then removes the released descendant.
    if LEADER_EXIT_ENDS_OUTPUT {
        receive_through_exit(&link, &mut messages);
        kill_released_group(&spawned);
    } else {
        let early = lifecycle(messages.clone());
        assert!(
            matches!(early.as_slice(), []),
            "published before EOF: {early:?}"
        );
        holder
            .signal(&Signal {
                target: target_of(&spawned),
                signal: SignalKind::Kill,
            })
            .expect("the reaped leader's group is still addressable");
        receive_through_exit(&link, &mut messages);
    }

    // Then: one Hangup names the spawned PID and start token at the final offset,
    // it precedes the Exit carrying the leader's status, nothing follows Exit,
    // and the descendant is gone.
    let events = lifecycle(messages);
    let [Ok((hangup, offset)), Err(exit)] = events.as_slice() else {
        panic!("expected exactly Hangup then Exit, got {events:?}");
    };
    assert_eq!(hangup.pid.get(), spawned.pid);
    assert_eq!(hangup.start_token, spawned.start_token);
    assert_eq!(*offset, exit.final_offset);
    assert_eq!((exit.code, exit.signal), (Some(3), None));
    assert!(matches!(
        link.recv_timeout(Duration::ZERO),
        Err(RecvError::Timeout)
    ));
    descendant.wait_exited();
    holder.shutdown().expect("shutdown");
}

#[test]
fn shutdown_with_descendant_holding_slave_is_bounded_and_keeps_hangup_before_exit() {
    // Given: the leader is reaped and a HUP-ignoring same-group descendant holds the slave.
    let (holder, link, spawned, descendant) = reaped_leader("t-reap-descendant");
    let mut messages = queued(&link);

    // When: the hangup stage runs after the reap, then shutdown runs.
    if LEADER_EXIT_ENDS_OUTPUT {
        receive_through_exit(&link, &mut messages);
        assert!(matches!(
            holder.hangup(&target_of(&spawned)),
            Err(HolderError::ProcessExited)
        ));
    } else {
        holder
            .hangup(&target_of(&spawned))
            .expect("the hangup stage reaches the reaped leader's live group");
    }
    let started = Instant::now();
    let report = holder.shutdown().expect("shutdown");
    let elapsed = started.elapsed();
    if LEADER_EXIT_ENDS_OUTPUT {
        kill_released_group(&spawned);
    }

    // Then: shutdown returns within budget with every worker joined, the leader
    // counts as already exited, delivered lifecycle events keep Hangup before a
    // final Exit, the exit is recorded at the tail head, and the descendant is gone.
    assert!(elapsed < SHUTDOWN_BUDGET, "shutdown took {elapsed:?}");
    assert_eq!(report.already_exited, 1);
    let joined = report.terminals.get(&spawned.terminal_id).expect("report");
    assert!(joined.all_joined());
    messages.extend(queued(&link));
    let events = lifecycle(messages);
    assert!(
        matches!(events.as_slice(), [] | [Ok(_)] | [Ok(_), Err(_)]),
        "lifecycle order violated: {events:?}"
    );
    let state = holder.shared.lock();
    let terminal = state.terminals.get(&spawned.terminal_id).expect("retained");
    let exit = terminal.held.exit.as_ref().expect("exit recorded");
    assert_eq!(exit.final_offset, terminal.tail.head());
    assert_eq!((exit.code, exit.signal), (Some(3), None));
    drop(state);
    descendant.wait_exited();
}

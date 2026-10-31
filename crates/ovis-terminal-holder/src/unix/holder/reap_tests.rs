//! Hangup is attributed from retained incarnation evidence, never from a reap barrier.
//!
//! A same-group descendant keeps the PTY slave open after the leader exits, so
//! the waiter reaps before the descendant is gone. Hangup must still name the
//! spawned incarnation and still precede Exit.

use std::time::{Duration, Instant};

use super::Holder;
use super::shutdown_tests::wait_until_reaped;
use crate::protocol::{
    Exit, HOLDER_PROTOCOL_GENERATION, Hangup, Message, Signal, SignalKind, Spawned,
};
use crate::unix::test_fixtures::{WAIT, config, hello_record, shell, target_of};
use crate::unix::{CoreLink, RecvError};

const SHUTDOWN_BUDGET: Duration = Duration::from_secs(2);
const LEADER_EXIT: &str = "trap '' HUP; sleep 777 & printf descendant-ready; exit 3";

/// Spawns `LEADER_EXIT` over an adopted link and returns once its leader is reaped.
fn reaped_leader(terminal_id: &str) -> (Holder, CoreLink, Spawned) {
    let holder = Holder::new(config());
    let link = holder
        .adopt(&hello_record(HOLDER_PROTOCOL_GENERATION))
        .expect("adopt");
    assert!(matches!(link.recv_timeout(WAIT), Ok(Message::Welcome(_))));
    let spawned = holder
        .spawn_wire(&shell(terminal_id, LEADER_EXIT))
        .expect("spawn");
    let mut seen = Vec::new();
    while !seen.windows(16).any(|bytes| bytes == b"descendant-ready") {
        match link.recv_timeout(WAIT).expect("descendant marker") {
            Message::Output(output) => seen.extend(output.bytes),
            Message::Spawned(_) => {}
            other => panic!("unexpected {:?}", other.kind()),
        }
    }
    wait_until_reaped(&holder, &spawned.terminal_id);
    (holder, link, spawned)
}

/// Messages already queued on `link`, without waiting for more.
fn queued(link: &CoreLink) -> Vec<Message> {
    std::iter::from_fn(|| link.recv_timeout(Duration::ZERO).ok()).collect()
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

/// When the master reports EOF depends on the platform. Linux keeps the master
/// open until the last slave descriptor closes, so nothing can be published
/// while the descendant lives. Darwin reports EOF as soon as the session leader
/// exits, even while a same-group descendant still holds the slave, so one
/// Hangup may already be queued at the reap. Either way exactly one Hangup names
/// the spawned incarnation and precedes a final Exit.
#[test]
fn hangup_after_reap_carries_spawned_incarnation_and_precedes_exit() {
    // Given: the leader is reaped while a descendant still holds the PTY slave.
    let (holder, link, spawned) = reaped_leader("t-reap-first");
    let mut messages = queued(&link);
    let early = lifecycle(messages.clone());
    if cfg!(target_os = "linux") {
        assert!(
            matches!(early.as_slice(), []),
            "published before EOF: {early:?}"
        );
    } else {
        assert!(
            matches!(early.as_slice(), [] | [Ok(_)]),
            "Exit published while the descendant lives: {early:?}"
        );
    }

    // When: the descendant's group is killed, closing the last slave reference.
    holder
        .signal(&Signal {
            target: target_of(&spawned),
            signal: SignalKind::Kill,
        })
        .expect("the reaped leader's group is still addressable");
    loop {
        let message = link.recv_timeout(WAIT).expect("lifecycle event");
        let exited = matches!(message, Message::Exit(_));
        messages.push(message);
        if exited {
            break;
        }
    }

    // Then: across both phases, one Hangup names the spawned PID and start token
    // at the final offset, it precedes the Exit carrying the leader's status, and
    // Exit is last.
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
    holder.shutdown().expect("shutdown");
}

#[test]
fn shutdown_with_descendant_holding_slave_is_bounded_and_keeps_hangup_before_exit() {
    // Given: the leader is reaped and a HUP-ignoring same-group descendant holds the slave.
    let (holder, link, spawned) = reaped_leader("t-reap-descendant");

    // When: the hangup stage runs after the reap, then shutdown kills the group.
    holder
        .hangup(&target_of(&spawned))
        .expect("the hangup stage reaches the reaped leader's live group");
    let started = Instant::now();
    let report = holder.shutdown().expect("shutdown");
    let elapsed = started.elapsed();

    // Then: shutdown returns within budget with every worker joined, the leader
    // counts as already exited, any lifecycle events delivered before the link
    // closed keep Hangup before a final Exit, and the exit is still recorded.
    assert!(elapsed < SHUTDOWN_BUDGET, "shutdown took {elapsed:?}");
    assert_eq!(report.already_exited, 1);
    let joined = report.terminals.get(&spawned.terminal_id).expect("report");
    assert!(joined.all_joined());
    let events = lifecycle(queued(&link));
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
}

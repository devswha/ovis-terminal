//! Holder socket service under disconnects, re-adoption and hostile frames.

use super::test_fixtures::{WAIT, adopt, hello, input, shell, spawn, start_service};
use super::{CoreConnection, CoreConnectionError};
use crate::process::{ProcessId, read_start_token};
use crate::protocol::{Drain, Goodbye, GoodbyeReason, Message, SpawnFailureKind};

fn shutdown(core: &mut CoreConnection) {
    let reason = GoodbyeReason::Shutdown;
    core.send(&Message::Goodbye(Goodbye { reason }))
        .expect("goodbye");
}

#[test]
fn repeated_readoption_keeps_pid_and_start_token() {
    // Given: a gated program owned by the holder.
    let (service, socket) = start_service();
    let (mut core, _) = adopt(&socket);
    let spawned = spawn(&mut core, shell("t-gated", "stty -echo; read gate"));

    // When: the core disconnects and re-adopts three times.
    for _ in 0..3 {
        drop(core);
        let (next, held) = adopt(&socket);
        core = next;

        // Then: every welcome shows the same live process.
        let record = held.first().expect("held terminal");
        assert_eq!(record.pid, Some(spawned.pid));
        assert_eq!(record.start_token.as_ref(), Some(&spawned.start_token));
        assert_eq!(record.exit, None);
    }
    let pid = ProcessId::new(spawned.pid);
    assert_eq!(read_start_token(pid).expect("alive"), spawned.start_token);

    // When: the last core shuts the holder down, its program ends with it.
    shutdown(&mut core);
    service.wait().expect("joined shutdown");
    assert_eq!(
        read_start_token(pid).expect_err("reaped").kind(),
        "process_gone"
    );
}

#[test]
fn stop_wakes_accept_and_joins_while_a_core_is_connected() {
    // Given: a connected core and a held program.
    let (service, socket) = start_service();
    let (mut core, _) = adopt(&socket);
    let spawned = spawn(&mut core, shell("t-stop", "stty -echo; read gate"));

    // When: the owner stops the service with the core still connected.
    service.stop().expect("stop joins every thread");

    // Then: the core sees the link end and the program is gone.
    loop {
        match core.recv(WAIT) {
            Ok(Message::Output(_)) => {}
            Err(CoreConnectionError::Closed) => break,
            other => panic!("unexpected {other:?}"),
        }
    }
    let pid = ProcessId::new(spawned.pid);
    assert_eq!(
        read_start_token(pid).expect_err("reaped").kind(),
        "process_gone"
    );
}

#[test]
fn malformed_frame_drops_only_that_connection() {
    // Given: a holder holding one program.
    let (service, socket) = start_service();
    let (mut core, _) = adopt(&socket);
    let spawned = spawn(&mut core, shell("t-kept", "stty -echo; read gate"));
    drop(core);

    // When: a connection sends a header with bad magic.
    let mut hostile = CoreConnection::connect(&socket).expect("connect");
    hostile
        .send_raw(b"BAD!\x01\x00\x01\x00\x00\x00\x00\x00")
        .expect("send garbage");
    let ended = hostile.recv(WAIT).expect_err("connection closes");

    // Then: only that connection ended, with a typed, queryable reason.
    assert_eq!(ended.kind(), "holder_closed");
    let (mut core, held) = adopt(&socket);
    assert_eq!(service.last_failure_kind(), Some("malformed_frame"));
    assert_eq!(
        held.first().and_then(|record| record.pid),
        Some(spawned.pid)
    );
    shutdown(&mut core);
    service.wait().expect("joined shutdown");
}

#[test]
fn requests_before_hello_and_unknown_generations_change_nothing() {
    // Given: a holder with no terminals.
    let (service, socket) = start_service();

    // When: a spawn arrives before any hello.
    let mut early = CoreConnection::connect(&socket).expect("connect");
    early
        .send(&Message::Spawn(shell("t-early", "true")))
        .expect("send");

    // Then: the connection ends with hello_required and nothing was spawned.
    assert!(matches!(early.recv(WAIT), Err(CoreConnectionError::Closed)));
    let (core, held) = adopt(&socket);
    assert_eq!(service.last_failure_kind(), Some("hello_required"));
    assert_eq!(held, Vec::new());
    drop(core);

    // When: a core offers generation 999.
    let mut future = CoreConnection::connect(&socket).expect("connect");
    future.send(&hello(999)).expect("send");

    // Then: no welcome, no ownership change, and a typed reason.
    assert!(matches!(
        future.recv(WAIT),
        Err(CoreConnectionError::Closed)
    ));
    let (mut core, held) = adopt(&socket);
    assert_eq!(service.last_failure_kind(), Some("unsupported_generation"));
    assert_eq!(held, Vec::new());
    shutdown(&mut core);
    service.wait().expect("joined shutdown");
}

#[test]
fn draining_holder_refuses_spawns_and_completes_once_empty() {
    // Given: an adopted holder with no terminals.
    let (service, socket) = start_service();
    let (mut core, _) = adopt(&socket);

    // When: the core begins a drain and then asks for a new terminal.
    core.send(&Message::Drain(Drain::Begin)).expect("drain");
    core.send(&Message::Spawn(shell("t-late", "true")))
        .expect("spawn");

    // Then: the spawn is refused as a typed outcome and completion stops the service.
    let Message::SpawnFailed(failed) = core.recv(WAIT).expect("outcome") else {
        panic!("a draining holder refuses spawns");
    };
    assert_eq!(failed.kind, SpawnFailureKind::Unavailable);
    core.send(&Message::Drain(Drain::Complete))
        .expect("complete");
    service.wait().expect("drained service stops");
}

#[test]
fn maximum_counters_round_trip_exactly_through_the_socket() {
    // Given: a program that reads one line.
    let (service, socket) = start_service();
    let (mut core, _) = adopt(&socket);
    let spawned = spawn(&mut core, shell("t-max", "stty -echo; read line"));

    // When: input carries u64::MAX sequence and epochs.
    let mut request = input(&spawned, u64::MAX, u64::MAX);
    request.bytes = b"x\n".to_vec();
    core.send(&Message::Input(request)).expect("input");

    // Then: the acknowledgement decodes to exactly the same counters.
    let ack = loop {
        if let Message::InputWritten(ack) = core.recv(WAIT).expect("ack") {
            break ack;
        }
    };
    assert_eq!(
        (ack.sequence, ack.grant_epoch, ack.fence_epoch),
        (u64::MAX, u64::MAX, u64::MAX)
    );
    assert_eq!(ack.written_bytes, 2);
    shutdown(&mut core);
    service.wait().expect("joined shutdown");
}

//! In-process holder control under a program that never reads its input.

use super::test_fixtures::{WAIT, config, hello_record, input, shell, target_of};
use super::{CoreLink, Holder};
use crate::protocol::{
    HOLDER_PROTOCOL_GENERATION, Message, ProcessIncarnation, Resize, Signal, SignalKind,
};

fn adopt(holder: &Holder) -> CoreLink {
    holder
        .adopt(&hello_record(HOLDER_PROTOCOL_GENERATION))
        .expect("adopt")
}

#[test]
fn newer_fence_revokes_queued_input_behind_a_blocked_write() {
    // Given: a raw-mode program that never reads, and a writer blocked on 1 MiB.
    let holder = Holder::new(config());
    let link = adopt(&holder);
    let quiet = shell("t-fence", "stty raw -echo; echo ready; exec sleep 600");
    let spawned = holder.spawn_wire(&quiet).expect("spawn");
    let mut seen = Vec::new();
    while !seen.windows(5).any(|window| window == b"ready") {
        if let Message::Output(output) = link.recv_timeout(WAIT).expect("output") {
            seen.extend(output.bytes);
        }
    }
    let mut blocked = input(&spawned, 1, 1);
    blocked.bytes = vec![b'x'; 1 << 20];
    holder.input(blocked).expect("first entry");
    let mut queued = input(&spawned, 2, 1);
    queued.bytes = b"stale".to_vec();
    holder.input(queued).expect("queued entry");

    // When: input under a newer fence epoch arrives.
    let mut fresh = input(&spawned, 3, 2);
    fresh.bytes = b"fresh".to_vec();
    holder.input(fresh).expect("fresh entry");

    // Then: the queued entry is revoked unwritten at once, though nothing was read;
    // the only earlier acknowledgement possible is entry 1 revoked before the
    // writer took it.
    loop {
        match link.recv_timeout(WAIT).expect("ack") {
            Message::InputWritten(ack) if ack.sequence == 2 => {
                assert_eq!((ack.written_bytes, ack.fence_epoch), (0, 2));
                break;
            }
            Message::InputWritten(ack) => {
                assert_eq!(
                    (ack.sequence, ack.written_bytes, ack.fence_epoch),
                    (1, 0, 2)
                );
            }
            Message::Output(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    let (target, signal) = (target_of(&spawned), SignalKind::Kill);
    holder.signal(&Signal { target, signal }).expect("kill");
    holder.shutdown().expect("joined");
}

#[test]
fn interrupt_delivery_is_not_termination_evidence() {
    // Given: a program that ignores interrupts and waits for a line.
    let holder = Holder::new(config());
    let link = adopt(&holder);
    let script = "trap '' INT; stty -echo; echo armed; read gate; exit 0";
    let spawned = holder.spawn_wire(&shell("t-int", script)).expect("spawn");
    let mut seen = Vec::new();
    while !seen.windows(5).any(|window| window == b"armed") {
        if let Message::Output(output) = link.recv_timeout(WAIT).expect("output") {
            seen.extend(output.bytes);
        }
    }

    // When: an interrupt is delivered and a newer then an older geometry arrive.
    let (target, signal) = (target_of(&spawned), SignalKind::Interrupt);
    holder
        .signal(&Signal { target, signal })
        .expect("delivered");
    let (target, geometry) = (target_of(&spawned), (100, 30));
    holder
        .resize(&Resize {
            target,
            geometry,
            geometry_epoch: 2,
        })
        .expect("resize");
    let (target, geometry) = (target_of(&spawned), (90, 20));
    let stale = holder
        .resize(&Resize {
            target,
            geometry,
            geometry_epoch: 1,
        })
        .expect_err("older epoch");

    // Then: the program still runs; its exit is reported only when it really exits.
    assert_eq!(stale.kind(), "stale_geometry");
    let mut gate = input(&spawned, 1, 1);
    gate.bytes = b"go\n".to_vec();
    holder.input(gate).expect("release gate");
    let exit = loop {
        if let Message::Exit(exit) = link.recv_timeout(WAIT).expect("exit") {
            break exit;
        }
    };
    assert_eq!((exit.code, exit.signal), (Some(0), None));
    holder.shutdown().expect("joined");
}

#[test]
fn stale_process_incarnation_and_sequence_regressions_are_rejected() {
    // Given: a gated program.
    let holder = Holder::new(config());
    let _link = adopt(&holder);
    let spawned = holder
        .spawn_wire(&shell("t-seq", "stty -echo; read gate"))
        .expect("spawn");
    holder.input(input(&spawned, 5, 1)).expect("first input");

    // When / Then: a repeated sequence and a foreign process incarnation are typed rejections.
    let repeated = holder
        .input(input(&spawned, 5, 1))
        .expect_err("sequence regression");
    assert_eq!(repeated.kind(), "input_sequence_regression");
    let mut foreign = input(&spawned, 6, 1);
    foreign.grant.process_incarnation = ProcessIncarnation::new(99);
    assert_eq!(
        holder.input(foreign).expect_err("stale").kind(),
        "stale_target"
    );
    holder.shutdown().expect("joined");
}

#[test]
fn no_worker_thread_survives_kill_reap_and_shutdown() {
    // Given: a non-reading program with a blocked input writer.
    let holder = Holder::new(config());
    let link = adopt(&holder);
    let quiet = shell("t-threads", "stty raw -echo; echo ready; exec sleep 600");
    let spawned = holder.spawn_wire(&quiet).expect("spawn");
    let mut seen = Vec::new();
    while !seen.windows(5).any(|window| window == b"ready") {
        if let Message::Output(output) = link.recv_timeout(WAIT).expect("output") {
            seen.extend(output.bytes);
        }
    }
    let mut blocked = input(&spawned, 1, 1);
    blocked.bytes = vec![b'x'; 1 << 20];
    holder.input(blocked).expect("blocked entry");
    assert_eq!(
        holder.handle_count(),
        4,
        "the waiter, pump and writer each hold the state"
    );

    // When: the program is killed, its exit is observed, and the holder shuts down.
    let (target, signal) = (target_of(&spawned), SignalKind::Kill);
    holder.signal(&Signal { target, signal }).expect("kill");
    while !matches!(link.recv_timeout(WAIT).expect("exit"), Message::Exit(_)) {}
    holder.shutdown().expect("joined");

    // Then: every worker thread has ended and released the holder state.
    assert_eq!(holder.handle_count(), 1);
}

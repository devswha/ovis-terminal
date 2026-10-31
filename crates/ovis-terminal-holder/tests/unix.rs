//! Unix PTY holder behavior through the real holder socket.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::error::Error;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use ovis_terminal_holder::process::{ProcessId, read_start_token};
use ovis_terminal_holder::protocol::{
    CoreBuildId, Exit, Forget, Goodbye, GoodbyeReason, HOLDER_PROTOCOL_GENERATION, HeldTerminal,
    Hello, HolderBuildId, HolderDescriptor, HolderInstanceId, Input, InputRejection, Message,
    PreviousCoreState, Resize, Signal, SignalKind, Spawn, SpawnFailureKind, SpawnFailureReason,
    Spawned, TailRequest, TerminalId, TerminalIncarnation, TerminalTarget, WriteGrant,
};
use ovis_terminal_holder::unix::{
    CoreConnection, Holder, HolderConfig, HolderLimits, HolderService,
};

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

const WAIT: Duration = Duration::from_secs(30);
const MIB: usize = 1 << 20;

fn config() -> HolderConfig {
    let build = HolderBuildId::new("holder-test-build".to_owned());
    let instance = HolderInstanceId::new(format!("holder-test-{}", std::process::id()));
    HolderConfig::new(
        HolderDescriptor::new(build, instance),
        HolderLimits::DEFAULT,
    )
}

fn start() -> Outcome<(HolderService, PathBuf)> {
    let socket = socket_path();
    let service = HolderService::start(UnixListener::bind(&socket)?, Holder::new(config()))?;
    Ok((service, socket))
}

fn socket_path() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ovis-t6-{}-{serial}.sock", std::process::id()))
}

fn hello() -> Hello {
    let core_build = CoreBuildId::new("core-test-build".to_owned());
    let (generation, previous) = (HOLDER_PROTOCOL_GENERATION, PreviousCoreState::Crash);
    Hello {
        generation,
        core_build,
        previous,
    }
}

fn shell(terminal: &str, script: &str) -> Spawn {
    Spawn {
        terminal_id: TerminalId::new(terminal.to_owned()),
        terminal_incarnation: TerminalIncarnation::new(1),
        argv: vec!["/bin/sh".to_owned(), "-c".to_owned(), script.to_owned()],
        cwd: std::env::temp_dir(),
        env: [("PATH", "/usr/bin:/bin"), ("TERM", "xterm")]
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
        geometry: (80, 24),
    }
}

fn target_of(spawned: &Spawned) -> TerminalTarget {
    TerminalTarget {
        terminal_id: spawned.terminal_id.clone(),
        terminal_incarnation: spawned.terminal_incarnation,
        process_incarnation: Some(spawned.process_incarnation),
    }
}

fn input(spawned: &Spawned, sequence: u64, bytes: Vec<u8>) -> Input {
    let process_incarnation = spawned.process_incarnation;
    let grant = WriteGrant {
        grant_epoch: 1,
        fence_epoch: 1,
        process_incarnation,
    };
    Input {
        target: target_of(spawned),
        sequence,
        grant,
        bytes,
    }
}

fn adopt(socket: &Path) -> Outcome<(CoreConnection, Vec<HeldTerminal>)> {
    let mut core = CoreConnection::connect(socket)?;
    core.send(&Message::Hello(hello()))?;
    match core.recv(WAIT)? {
        Message::Welcome(welcome) => Ok((core, welcome.terminals)),
        other => Err(format!("expected Welcome, got {other:?}").into()),
    }
}

/// Reads until `Exit`, requiring output to stay contiguous from `offset`.
fn output_until_exit(core: &mut CoreConnection, mut offset: u64) -> Outcome<(Vec<u8>, Exit)> {
    let mut bytes = Vec::new();
    loop {
        match core.recv(WAIT)? {
            Message::Output(output) if output.offset == offset => {
                offset = offset
                    .checked_add(u64::try_from(output.bytes.len())?)
                    .ok_or("offset")?;
                bytes.extend(output.bytes);
            }
            Message::InputWritten(ack) if ack.written_bytes == 3 => {}
            Message::Exit(exit) if exit.final_offset == offset => return Ok((bytes, exit)),
            other => return Err(format!("out of order at offset {offset}: {other:?}").into()),
        }
    }
}

#[test]
fn happy() -> Outcome {
    // Given: a holder service, and a child that prints the first byte of "é" and waits.
    let (service, socket) = start()?;
    let (mut first, held) = adopt(&socket)?;
    assert_eq!(held, Vec::new());
    let script = "stty -echo; printf '\\303'; read gate; printf '\\251\\n'; exit 7";
    first.send(&Message::Spawn(shell("t-happy", script)))?;
    let Message::Spawned(spawned) = first.recv(WAIT)? else {
        return Err("Spawned must precede any output".into());
    };
    let Message::Output(lead) = first.recv(WAIT)? else {
        return Err("expected the lead byte".into());
    };
    assert_eq!((lead.offset, lead.bytes), (0, vec![0xC3]));

    // When: the core disconnects mid-character and another core re-adopts.
    drop(first);
    let (mut second, held) = adopt(&socket)?;

    // Then: the same process is held, with its PID and start token unchanged.
    let record = held.first().ok_or("retained terminal")?;
    assert_eq!(record.pid, Some(spawned.pid));
    assert_eq!(record.start_token.as_ref(), Some(&spawned.start_token));
    assert_eq!((record.output_offset, record.exit.is_none()), (1, true));
    assert_eq!(
        read_start_token(ProcessId::new(spawned.pid))?,
        spawned.start_token
    );

    // When: the new core releases the gate, the child completes the character and exits.
    second.send(&Message::Input(input(&spawned, 1, b"go\n".to_vec())))?;
    let (rest, exit) = output_until_exit(&mut second, 1)?;

    // Then: final bytes precede exit, and the retained tail spans the disconnect.
    assert_eq!(rest, vec![0xA9, b'\r', b'\n']);
    assert_eq!(
        (exit.code, exit.signal, exit.final_offset),
        (Some(7), None, 4)
    );
    second.send(&Message::TailRequest(TailRequest {
        target: target_of(&spawned),
        from_offset: 0,
    }))?;
    let next = second.recv(WAIT)?;
    let Message::Tail(tail) = next else {
        return Err(format!("no output may follow Exit: got {:?}", next.kind()).into());
    };
    assert_eq!(
        (tail.from_offset, tail.through_offset, tail.gap),
        (0, 4, false)
    );
    assert_eq!(String::from_utf8(tail.bytes)?, "é\r\n");
    let (target, acknowledged_final_offset) = (target_of(&spawned), Some(4));
    second.send(&Message::Forget(Forget {
        target,
        acknowledged_final_offset,
    }))?;
    let holder = service.holder().clone();
    shut_down(&mut second, service, &socket)?;
    assert!(holder.is_empty());
    Ok(())
}

fn shut_down(core: &mut CoreConnection, service: HolderService, socket: &Path) -> Outcome {
    let reason = GoodbyeReason::Shutdown;
    core.send(&Message::Goodbye(Goodbye { reason }))?;
    service.wait()?;
    Ok(std::fs::remove_file(socket)?)
}

#[test]
fn failure() -> Outcome {
    // Given: a holder service and an adopted core.
    let (service, socket) = start()?;
    let (mut core, _) = adopt(&socket)?;

    // When: argv names a missing executable.
    let missing = Spawn {
        argv: vec!["/nonexistent/ovis-fixture".to_owned()],
        ..shell("t-x", "")
    };
    core.send(&Message::Spawn(missing))?;

    // Then: a typed spawn failure carrying the OS error, and no terminal record.
    let Message::SpawnFailed(failed) = core.recv(WAIT)? else {
        return Err("expected SpawnFailed".into());
    };
    let expected = (
        SpawnFailureKind::InvalidRequest,
        SpawnFailureReason::ExecutableUnavailable,
    );
    assert_eq!((failed.kind, failed.reason), expected);
    assert_eq!(failed.os_error, Some(libc::ENOENT));
    assert_ne!(failed.hint, "");

    // Given: a raw-mode child that never reads stdin.
    let quiet = shell("t-full", "stty raw -echo; echo ready; exec sleep 600");
    core.send(&Message::Spawn(quiet))?;
    let Message::Spawned(spawned) = core.recv(WAIT)? else {
        return Err("expected Spawned".into());
    };
    let mut seen = Vec::new();
    while !seen.windows(5).any(|window| window == b"ready") {
        if let Message::Output(output) = core.recv(WAIT)? {
            seen.extend(output.bytes);
        }
    }

    // When: twenty 1 MiB inputs are sent through the socket.
    for sequence in 1..=20 {
        core.send(&Message::Input(input(&spawned, sequence, vec![b'x'; MIB])))?;
    }

    // Then: the holder answers with a typed input_overflow carrying its hint and bounds.
    let rejected = loop {
        match core.recv(WAIT)? {
            Message::InputRejected(rejected) => break rejected,
            Message::Output(_) => {}
            other => return Err(format!("expected InputRejected, got {other:?}").into()),
        }
    };
    assert_eq!(rejected.reason, InputRejection::InputOverflow);
    assert!(
        rejected.sequence <= 18,
        "one entry blocks in the PTY and 16 MiB queue behind it"
    );
    assert_eq!(
        (rejected.limit_bytes, rejected.limit_entries),
        (16 << 20, 4096)
    );
    assert!(rejected.queued_bytes.saturating_add(1 << 20) > rejected.limit_bytes);
    assert_ne!(rejected.hint, "");

    // Then: resize and kill still settle over the same socket.
    let (target, geometry, geometry_epoch) = (target_of(&spawned), (120, 40), 1);
    core.send(&Message::Resize(Resize {
        target,
        geometry,
        geometry_epoch,
    }))?;
    let (target, signal) = (target_of(&spawned), SignalKind::Kill);
    core.send(&Message::Signal(Signal { target, signal }))?;
    let admitted = rejected.sequence.saturating_sub(1);
    let (mut acks, mut exit) = (BTreeMap::new(), None);
    while exit.is_none() || acks.len() < usize::try_from(admitted)? {
        match core.recv(WAIT)? {
            Message::InputWritten(ack) => {
                assert_eq!(acks.insert(ack.sequence, ack.written_bytes), None);
            }
            Message::InputRejected(later) => assert!(later.sequence > rejected.sequence),
            Message::Exit(record) => exit = Some(record),
            _ => {}
        }
    }
    let exit = exit.ok_or("exit")?;
    assert_eq!(
        (exit.signal, exit.exit_unknown, exit.io_failure),
        (Some(9), None, None)
    );
    let partial = acks.get(&1).copied().ok_or("first entry acknowledged")?;
    assert!(
        partial > 0 && partial < 1 << 20,
        "a partial write is reported as written"
    );
    let total: u64 = acks.values().sum();
    assert!(total < admitted.saturating_mul(1 << 20));
    shut_down(&mut core, service, &socket)
}

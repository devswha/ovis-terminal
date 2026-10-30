//! Unix PTY holder behavior through the holder socket and the in-process API.
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
    Hello, HolderBuildId, HolderDescriptor, HolderInstanceId, Input, Message, PreviousCoreState,
    Resize, Signal, SignalKind, Spawn, SpawnFailureKind, Spawned, TailRequest, TerminalId,
    TerminalIncarnation, TerminalTarget, WriteGrant,
};
use ovis_terminal_holder::unix::{
    CoreConnection, Holder, HolderConfig, HolderError, HolderLimits, HolderService,
    ProcessEnvironment,
};

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

const WAIT: Duration = Duration::from_secs(30);
const MIB: usize = 1 << 20;

fn config() -> HolderConfig {
    let build = HolderBuildId::new("holder-test-build".to_owned());
    let instance = HolderInstanceId::new(format!("holder-test-{}", std::process::id()));
    let vars = vec![
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("TERM".into(), "xterm".into()),
    ];
    let environment = ProcessEnvironment::new(std::env::temp_dir(), vars);
    HolderConfig::new(
        HolderDescriptor::new(build, instance),
        HolderLimits::DEFAULT,
        environment,
    )
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
    let socket = socket_path();
    let service = HolderService::start(UnixListener::bind(&socket)?, Holder::new(config()))?;
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
    let (target, from_offset) = (target_of(&spawned), 0);
    second.send(&Message::TailRequest(TailRequest {
        target,
        from_offset,
    }))?;
    let Message::Tail(tail) = second.recv(WAIT)? else {
        return Err("no output may follow Exit".into());
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
    let reason = GoodbyeReason::Shutdown;
    second.send(&Message::Goodbye(Goodbye { reason }))?;
    let holder = service.holder().clone();
    service.wait()?;
    assert!(holder.is_empty());
    Ok(std::fs::remove_file(&socket)?)
}

#[test]
fn failure() -> Outcome {
    // Given: an adopted in-process holder.
    let holder = Holder::new(config());
    let link = holder.adopt(&hello())?;
    assert!(matches!(link.recv_timeout(WAIT)?, Message::Welcome(_)));

    // When: argv names a missing executable.
    let missing = Spawn {
        argv: vec!["/nonexistent/ovis-fixture".to_owned()],
        ..shell("t-x", "")
    };
    let Err(error) = holder.spawn_wire(&missing) else {
        return Err("a missing executable must not spawn".into());
    };

    // Then: a typed spawn failure, reported separately, and no terminal record.
    assert_eq!(error.kind(), "spawn_failed");
    let HolderError::Launch { source } = &error else {
        return Err(format!("expected a launch failure, got {error:?}").into());
    };
    assert_eq!(source.kind(), "executable_rejected");
    let Message::SpawnFailed(failed) = link.recv_timeout(WAIT)? else {
        return Err("expected SpawnFailed".into());
    };
    assert_eq!(failed.kind, SpawnFailureKind::InvalidRequest);
    assert!(holder.is_empty());

    // Given: a raw-mode child that never reads stdin.
    let spawned = holder.spawn_wire(&shell(
        "t-full",
        "stty raw -echo; echo ready; exec sleep 600",
    ))?;
    let mut seen = Vec::new();
    while !seen.windows(5).any(|window| window == b"ready") {
        if let Message::Output(output) = link.recv_timeout(WAIT)? {
            seen.extend(output.bytes);
        }
    }

    // When: 1 MiB inputs are offered until the 16 MiB queue is full.
    let rejected = (1..=24_u64).find_map(|sequence| {
        let offered = holder.input(input(&spawned, sequence, vec![b'x'; MIB]));
        offered.err().map(|error| (sequence, error))
    });

    // Then: a typed input_overflow, while resize and kill still settle.
    let (overflowed, error) = rejected.ok_or("the bounded queue must reject input")?;
    assert_eq!(error.kind(), "input_overflow");
    assert!(matches!(error, HolderError::InputOverflow { limit, .. } if limit == 16 * MIB));
    assert!(
        overflowed <= 18,
        "one entry blocks in the PTY and 16 MiB queue behind it"
    );
    let (target, geometry, geometry_epoch) = (target_of(&spawned), (120, 40), 1);
    holder.resize(&Resize {
        target,
        geometry,
        geometry_epoch,
    })?;
    let (target, signal) = (target_of(&spawned), SignalKind::Kill);
    holder.signal(&Signal { target, signal })?;
    let (mut acks, mut exit) = (BTreeMap::new(), None);
    while exit.is_none() || acks.len() < usize::try_from(overflowed)? {
        match link.recv_timeout(WAIT)? {
            Message::InputWritten(ack) => {
                assert_eq!(acks.insert(ack.sequence, ack.written_bytes), None);
            }
            Message::Exit(record) => exit = Some(record),
            _ => {}
        }
    }
    assert_eq!(exit.and_then(|record| record.signal), Some(9));
    let partial = acks.get(&1).copied().ok_or("first entry acknowledged")?;
    assert!(
        partial > 0 && partial < 1 << 20,
        "a partial write is reported as written"
    );
    assert_eq!(acks.get(&overflowed).copied(), Some(0));
    let total: u64 = acks.values().sum();
    let queued = overflowed
        .saturating_sub(1)
        .saturating_mul(u64::try_from(MIB)?);
    assert!(
        total < queued,
        "the stopped program received only what the PTY accepted"
    );
    Ok(holder.shutdown()?)
}

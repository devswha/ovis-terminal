//! A real holder on a real named pipe and ConPTY, driven by a test core over the wire.

use std::sync::atomic::{AtomicU32, Ordering};

use ovis_terminal_protocol::domain::unix_service::{BootProof, Observed, StaleReason};
use ovis_terminal_protocol::domain::windows_service::{HolderPlacement, HostRecord};
use ovis_terminal_protocol::schema::ids::{ProcessPid, StartToken};

use super::facts::observe;
use super::{HolderHost, HostConfig, HostPaths, HostSession, pipe, write_record};
use crate::protocol::{Message, PreviousCoreState, Signal, SignalKind, TailRequest, TerminalId};

static NEXT: AtomicU32 = AtomicU32::new(0);
const MARKER: &str = "OVIS-54-MARKER";

fn paths() -> HostPaths {
    let unique = NEXT.fetch_add(1, Ordering::Relaxed);
    let session = HostSession::parse(&format!("t54-{}-{unique}", std::process::id())).unwrap();
    let state = std::env::temp_dir().join(format!("ovis-t54-{}-{unique}", std::process::id()));
    HostPaths::new(session, state).unwrap()
}

fn owner() -> String {
    pipe::current_user_sid().unwrap()
}

mod wire;

use wire::{Core, runtime, spawn, target};

#[test]
fn a_core_restart_re_adopts_the_same_incarnation_and_its_output_survives() {
    let paths = paths();
    let host = HolderHost::start(HostConfig::new(paths.clone(), HolderPlacement::Shared)).unwrap();
    let name = host.record().pipe().unwrap();
    assert!(matches!(
        observe(&paths, &owner()).unwrap().0,
        Observed::Running { .. }
    ));
    let script = format!("echo {MARKER}& ping -n 120 127.0.0.1 >NUL");
    let (spawned, through) = runtime().block_on(async {
        let mut first = Core::connect(&name);
        assert_eq!(
            first.adopt(PreviousCoreState::Crash).await.terminals,
            Vec::new()
        );
        first.send(spawn("t1", &script)).await;
        let Message::Spawned(spawned) = first.next().await else {
            panic!("no spawned")
        };
        first.output_until(&spawned, MARKER).await;
        drop(first);
        let mut second = Core::connect(&name);
        let welcome = second.adopt(PreviousCoreState::Crash).await;
        let [held] = welcome.terminals.as_slice() else {
            panic!("{welcome:?}")
        };
        assert_eq!(
            (held.pid, held.start_token.as_ref()),
            (Some(spawned.pid), Some(&spawned.start_token))
        );
        assert_eq!(held.process_incarnation, Some(spawned.process_incarnation));
        assert!(held.exit.is_none(), "the program ended with the first core");
        second
            .send(Message::TailRequest(TailRequest {
                target: target(&spawned),
                from_offset: 0,
            }))
            .await;
        let tail = loop {
            if let Message::Tail(tail) = second.next().await {
                break tail;
            }
        };
        assert!(String::from_utf8_lossy(&tail.bytes).contains(MARKER));
        second
            .send(Message::Signal(Signal {
                target: target(&spawned),
                signal: SignalKind::Kill,
            }))
            .await;
        let exit = loop {
            if let Message::Exit(exit) = second.next().await {
                break exit;
            }
        };
        assert!(exit.final_offset >= tail.through_offset, "{exit:?}");
        (spawned, exit.final_offset)
    });
    assert!(through > 0, "{spawned:?}");
    host.stop().unwrap();
    assert!(!paths.record().exists(), "the record outlived its holder");
    assert_eq!(observe(&paths, &owner()).unwrap().0, Observed::Stopped);
}

#[test]
fn a_dedicated_holder_owns_one_terminal_and_refuses_another() {
    let paths = paths();
    let host = HolderHost::start(HostConfig::new(paths, HolderPlacement::Dedicated)).unwrap();
    assert_eq!(host.record().placement, HolderPlacement::Dedicated);
    let name = host.record().pipe().unwrap();
    runtime().block_on(async {
        let mut core = Core::connect(&name);
        core.adopt(PreviousCoreState::Handover).await;
        core.send(spawn("one", "ping -n 120 127.0.0.1 >NUL")).await;
        assert!(matches!(core.next().await, Message::Spawned(_)));
        core.send(spawn("two", "ping -n 120 127.0.0.1 >NUL")).await;
        let refused = loop {
            match core.next().await {
                Message::SpawnFailed(failed) => break failed,
                Message::Output(_) => {}
                other => panic!("expected a refusal, got {other:?}"),
            }
        };
        assert_eq!(refused.terminal_id, TerminalId::new("two".to_owned()));
    });
    host.stop().unwrap();
}

#[test]
fn a_claimed_pipe_is_never_joined_and_a_dead_holder_record_is_stale() {
    let paths = paths();
    let host = HolderHost::start(HostConfig::new(paths.clone(), HolderPlacement::Shared)).unwrap();
    let again = HolderHost::start(HostConfig::new(paths.clone(), HolderPlacement::Shared));
    assert_eq!(
        again.map(drop).map_err(|e| e.kind()),
        Err("already_running")
    );
    let name = host.record().pipe().unwrap();
    host.stop().unwrap();
    let mut child = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "exit 0"])
        .spawn()
        .unwrap();
    let gone = child.id();
    child.wait().unwrap();
    let dead = BootProof::new(
        ProcessPid::new(gone),
        StartToken::new("windows-created:1".to_owned()),
    );
    write_record(
        &paths.record(),
        &HostRecord::new(paths.session().label(), &name, dead),
    )
    .unwrap();
    let observed = observe(&paths, &owner()).unwrap().0;
    assert!(
        matches!(
            observed,
            Observed::StaleEndpoint {
                reason: StaleReason::ProcessGone | StaleReason::ProcessReplaced,
                ..
            }
        ),
        "{observed:?}"
    );
    assert!(
        paths.record().exists(),
        "observing removed the stale record"
    );
    let replacement =
        HolderHost::start(HostConfig::new(paths.clone(), HolderPlacement::Shared)).unwrap();
    replacement.stop().unwrap();
    std::fs::remove_dir_all(paths.state_dir()).unwrap();
}

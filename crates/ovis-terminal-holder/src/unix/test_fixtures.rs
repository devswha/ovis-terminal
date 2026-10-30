//! Fixtures shared by the holder unit tests: configuration, socket service, core.

use std::collections::BTreeMap;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use super::{CoreConnection, Holder, HolderConfig, HolderLimits, HolderService};
use crate::protocol::{
    CoreBuildId, HOLDER_PROTOCOL_GENERATION, HeldTerminal, Hello, HolderBuildId, HolderDescriptor,
    HolderInstanceId, Input, Message, PreviousCoreState, Spawn, Spawned, TerminalId,
    TerminalIncarnation, TerminalTarget, WriteGrant,
};

pub(super) const WAIT: Duration = Duration::from_secs(30);

pub(super) fn config() -> HolderConfig {
    let build = HolderBuildId::new("holder-unit-build".to_owned());
    let instance = HolderInstanceId::new(format!("holder-unit-{}", std::process::id()));
    HolderConfig::new(
        HolderDescriptor::new(build, instance),
        HolderLimits::DEFAULT,
    )
}

/// The explicit environment every fixture launch carries.
pub(super) fn base_env() -> BTreeMap<String, String> {
    [("PATH", "/usr/bin:/bin"), ("TERM", "xterm")]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

pub(super) fn hello_record(generation: u16) -> Hello {
    let core_build = CoreBuildId::new("core-unit-build".to_owned());
    let previous = PreviousCoreState::Crash;
    Hello {
        generation,
        core_build,
        previous,
    }
}

pub(super) fn hello(generation: u16) -> Message {
    Message::Hello(hello_record(generation))
}

pub(super) fn shell(terminal: &str, script: &str) -> Spawn {
    Spawn {
        terminal_id: TerminalId::new(terminal.to_owned()),
        terminal_incarnation: TerminalIncarnation::new(1),
        argv: vec!["/bin/sh".to_owned(), "-c".to_owned(), script.to_owned()],
        cwd: std::env::temp_dir(),
        env: base_env(),
        geometry: (80, 24),
    }
}

pub(super) fn target_of(spawned: &Spawned) -> TerminalTarget {
    TerminalTarget {
        terminal_id: spawned.terminal_id.clone(),
        terminal_incarnation: spawned.terminal_incarnation,
        process_incarnation: Some(spawned.process_incarnation),
    }
}

/// Input whose grant and fence epochs are both `epoch`.
pub(super) fn input(spawned: &Spawned, sequence: u64, epoch: u64) -> Input {
    let grant = WriteGrant {
        grant_epoch: epoch,
        fence_epoch: epoch,
        process_incarnation: spawned.process_incarnation,
    };
    Input {
        target: target_of(spawned),
        sequence,
        grant,
        bytes: Vec::new(),
    }
}

pub(super) fn start_service() -> (HolderService, PathBuf) {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let name = format!("ovis-t6u-{}-{serial}.sock", std::process::id());
    let socket = std::env::temp_dir().join(name);
    let listener = UnixListener::bind(&socket).expect("bind holder socket");
    let service = HolderService::start(listener, Holder::new(config())).expect("start service");
    (service, socket)
}

pub(super) fn adopt(socket: &Path) -> (CoreConnection, Vec<HeldTerminal>) {
    let mut core = CoreConnection::connect(socket).expect("connect");
    core.send(&hello(HOLDER_PROTOCOL_GENERATION))
        .expect("hello");
    let Message::Welcome(welcome) = core.recv(WAIT).expect("welcome") else {
        panic!("the first holder message must be Welcome");
    };
    (core, welcome.terminals)
}

/// Spawns a shell script over `core` and returns its `Spawned` record.
pub(super) fn spawn(core: &mut CoreConnection, spawn: Spawn) -> Spawned {
    core.send(&Message::Spawn(spawn)).expect("spawn");
    loop {
        match core.recv(WAIT).expect("spawn outcome") {
            Message::Spawned(spawned) => return spawned,
            Message::Output(_) => {}
            other => panic!("unexpected spawn outcome {other:?}"),
        }
    }
}

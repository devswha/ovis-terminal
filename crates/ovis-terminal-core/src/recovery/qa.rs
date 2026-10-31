//! Recovery QA against real holders, real sockets and real engines.

mod crash;
mod durability;
mod exact;
mod groups;
mod reboot;
mod verdicts;

use std::os::unix::net::UnixListener;
use std::path::PathBuf;

use ovis_terminal_holder::process::{ProcessId, read_start_token};
use ovis_terminal_holder::protocol::{
    HolderBuildId, HolderDescriptor, HolderInstanceId, Message, Spawned, TerminalId,
    TerminalIncarnation, TerminalTarget,
};
use ovis_terminal_holder::unix::{
    Grid, Holder, HolderConfig, HolderLimits, HolderService, LaunchRequest, LaunchSpec,
    ProcessEnvironment,
};
use ovis_terminal_protocol::schema::ids::{RuntimeLifetimeId, ServerBootId};
use ovis_terminal_vt::{Geometry, HistoryLimit, VtConfig};

use super::{Adopted, GroupId, HolderGroup, Recorded, RecoveryPlan};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// One holder serving a socket in a private state directory.
struct Served {
    root: PathBuf,
    holder: Holder,
    service: Option<HolderService>,
    group: HolderGroup,
}

impl Served {
    fn new(root: &std::path::Path, group: GroupId) -> TestResult<Self> {
        let socket = root.join(format!("holder-{}.sock", uuid::Uuid::now_v7().simple()));
        let holder = Holder::new(HolderConfig::new(
            HolderDescriptor::new(
                HolderBuildId::new("task-49".into()),
                HolderInstanceId::new(uuid::Uuid::now_v7().to_string()),
            ),
            HolderLimits::DEFAULT,
        ));
        let service = HolderService::start(UnixListener::bind(&socket)?, holder.clone())?;
        Ok(Self {
            root: root.to_path_buf(),
            holder,
            service: Some(service),
            group: HolderGroup::new(group, socket),
        })
    }

    /// Starts `/bin/sh -c script` as terminal `id` and returns what the core records.
    fn spawn(&self, id: &str, script: &str) -> TestResult<Recorded> {
        let spec = LaunchSpec::new(
            vec!["/bin/sh".into(), "-c".into(), script.into()],
            ProcessEnvironment::new(
                self.root.clone(),
                vec![("PATH".into(), "/usr/bin:/bin".into())],
            ),
            Grid::new(80, 24)?,
        );
        let request = LaunchRequest::new(
            TerminalId::new(id.to_owned()),
            TerminalIncarnation::new(1),
            spec,
        );
        let Spawned {
            terminal_id,
            terminal_incarnation,
            process_incarnation,
            pid,
            start_token,
        } = self.holder.spawn(request)?;
        assert!(pid > 1, "only a spawned child is recorded");
        let target = TerminalTarget {
            terminal_id,
            terminal_incarnation,
            process_incarnation: Some(process_incarnation),
        };
        Ok(Recorded::new(
            target,
            (pid, start_token),
            self.group.id.clone(),
        ))
    }

    /// Makes the socket path unreachable while the holder and its programs keep running.
    fn unplug(&self) -> TestResult {
        std::fs::rename(
            &self.group.socket,
            self.group.socket.with_extension("hidden"),
        )?;
        Ok(())
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        let hidden = self.group.socket.with_extension("hidden");
        if hidden.exists() {
            let restored = std::fs::rename(&hidden, &self.group.socket);
            assert!(restored.is_ok(), "restore the owned socket: {restored:?}");
        }
        if let Some(service) = self.service.take() {
            let stopped = service.stop();
            assert!(stopped.is_ok(), "stop owned holder service: {stopped:?}");
        }
        let shut = self.holder.shutdown();
        assert!(shut.is_ok(), "join owned holder workers: {shut:?}");
    }
}

struct State(PathBuf);

impl State {
    fn new() -> TestResult<Self> {
        let root = std::env::temp_dir().join(format!("ovis-t49-{}", uuid::Uuid::now_v7().simple()));
        std::fs::create_dir_all(&root)?;
        Ok(Self(root))
    }

    fn plan(&self) -> TestResult<RecoveryPlan> {
        let config = VtConfig::new(Geometry::new(80, 24)?, HistoryLimit::new(2000));
        let identity = (
            RuntimeLifetimeId::new("runtime:49".to_owned()),
            ServerBootId::new("server-boot:2".to_owned()),
        );
        Ok(RecoveryPlan::new(self.0.clone(), identity, config))
    }
}

impl Drop for State {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.0);
        assert!(removed.is_ok(), "remove owned state: {removed:?}");
    }
}

/// Receives output of `target` from `link` until `marker` was seen; returns every byte.
fn until(link: &mut Adopted, (target, marker): (&TerminalTarget, &[u8])) -> TestResult<Vec<u8>> {
    let mut seen = Vec::new();
    while !seen.windows(marker.len()).any(|bytes| bytes == marker) {
        if let Message::Output(output) = link.receive()?
            && output.target.terminal_id == target.terminal_id
        {
            seen.extend_from_slice(&output.bytes);
        }
    }
    Ok(seen)
}

/// Whether the operating system still runs `recorded` with its start token.
fn running(recorded: &Recorded) -> bool {
    assert!(recorded.pid > 1, "only a spawned child is inspected");
    read_start_token(ProcessId::new(recorded.pid)).is_ok_and(|token| token == recorded.start_token)
}

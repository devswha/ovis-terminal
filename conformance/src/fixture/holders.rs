//! Real shared and dedicated holders behind a fixture runtime (plan todo 52).
//!
//! The shared holder of a session and the dedicated holder of one terminal each serve
//! their own Unix socket and are adopted and recovered by the core's recovery (todo 49).
//!
//! `fixture.holders` runs one drill per call in a fresh state directory: `exit` (each
//! placement's final output arrives before its exit), `crash` (a core lost after its last
//! checkpoint while the holder kept less than the rest: a gap, never exact), `reboot`
//! (another boot identity: restored metadata, nothing adopted) and `unplug` (a lost
//! dedicated holder makes only its own terminal unknown). Programs are never relaunched.

mod drills;

use std::os::unix::fs::DirBuilderExt as _;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use ovis_terminal_core::recovery::{GroupId, HolderGroup, Recorded};
use ovis_terminal_holder::protocol::{
    HolderBuildId, HolderDescriptor, HolderInstanceId, Spawned, TerminalId, TerminalIncarnation,
    TerminalTarget,
};
use ovis_terminal_holder::unix::{
    Grid, Holder, HolderConfig, HolderLimits, HolderService, LaunchRequest, LaunchSpec,
    ProcessEnvironment,
};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::decode_params;
use serde::Deserialize;
use serde_json::Value;

use super::machines::internal;

/// The unadvertised holder drill.
pub(crate) const HOLDERS: &str = "fixture.holders";

static NEXT: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    action: Drill,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Drill {
    Exit,
    Crash,
    Reboot,
    Unplug,
}

/// One real holder serving a socket in the drill's state directory.
#[derive(Debug)]
struct Served {
    root: PathBuf,
    holder: Holder,
    service: Option<HolderService>,
    group: HolderGroup,
}

impl Served {
    /// A holder for `group` that retains at most `tail` bytes per terminal.
    fn new(root: &Path, (group, tail): (GroupId, usize)) -> Result<Self, BoundaryError> {
        let id = uuid::Uuid::now_v7().simple().to_string();
        let socket = root.join(format!("h-{}.sock", id.get(24..).unwrap_or_default()));
        let defaults = HolderLimits::DEFAULT;
        let limits = HolderLimits::new(tail, defaults.input, defaults.link_bytes);
        let descriptor = HolderDescriptor::new(
            HolderBuildId::new("task-52".to_owned()),
            HolderInstanceId::new(uuid::Uuid::now_v7().to_string()),
        );
        let holder = Holder::new(HolderConfig::new(descriptor, limits));
        let listener = UnixListener::bind(&socket).map_err(internal)?;
        let service = HolderService::start(listener, holder.clone()).map_err(internal)?;
        Ok(Self {
            root: root.to_path_buf(),
            holder,
            service: Some(service),
            group: HolderGroup::new(group, socket),
        })
    }

    /// Starts `/bin/sh -c script` as terminal `id` and returns what a core records.
    fn spawn(&self, id: &str, script: &str) -> Result<Recorded, BoundaryError> {
        let environment = vec![("PATH".into(), "/usr/bin:/bin".into())];
        let spec = LaunchSpec::new(
            vec!["/bin/sh".into(), "-c".into(), script.into()],
            ProcessEnvironment::new(self.root.clone(), environment),
            Grid::new(80, 24).map_err(internal)?,
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
        } = self.holder.spawn(request).map_err(internal)?;
        if pid <= 1 {
            return Err(internal("the holder reported no spawned child"));
        }
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

    /// Makes the socket unreachable while the holder and its programs keep running.
    fn unplug(&self) -> Result<(), BoundaryError> {
        let hidden = self.group.socket.with_extension("hidden");
        std::fs::rename(&self.group.socket, hidden).map_err(internal)
    }

    /// Stops serving and ends the holder's programs and workers.
    fn close(mut self) -> Result<(), BoundaryError> {
        let hidden = self.group.socket.with_extension("hidden");
        if hidden.exists() {
            std::fs::rename(&hidden, &self.group.socket).map_err(internal)?;
        }
        if let Some(service) = self.service.take() {
            service.stop().map_err(internal)?;
        }
        self.holder.shutdown().map(drop).map_err(internal)
    }
}

/// `fixture.holders`: runs the named drill in a fresh owner-only directory below `root`.
pub(crate) async fn call(root: &Path, request: &RpcRequest) -> Result<Value, BoundaryError> {
    let params: Params = decode_params(request).map_err(BoundaryError::from)?;
    let dir = root.join(format!("h{}", NEXT.fetch_add(1, Ordering::SeqCst)));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(internal)?;
    tokio::task::spawn_blocking(move || drills::run(&dir, params.action))
        .await
        .map_err(internal)?
}

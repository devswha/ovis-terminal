//! The `update.*` lane on the fixture's dispatcher, over a real holder, restartable
//! control and install root, plus the unadvertised `fixture.update` probe.
//!
//! At serve time the fixture runs one held program and publishes, with the system `tar`,
//! a ready release `2.0.0`, a release `3.0.0` whose executable fails readiness, and a
//! release `4.0.0` whose archive climbs out of its root. The probe reports their digests
//! and whether any file escaped into the install root.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_authz::{
    Authority, EndpointPolicy, LocalAccount, PeerEvidence, Presentation, RemoteTransport, UnixUid,
};
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity};
use ovis_terminal_core::restart::{Control, ControlPlan};
use ovis_terminal_core::terminals::{Call, Launch, Program};
use ovis_terminal_holder::protocol::{HolderBuildId, HolderDescriptor, HolderInstanceId};
use ovis_terminal_holder::unix::{Holder, HolderConfig, HolderLimits};
use ovis_terminal_protocol::domain::update::{BuildTarget, HolderGeneration};
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind, OpId};
use ovis_terminal_protocol::schema::scalar::{ProtocolGeneration, TimestampMs};
use ovis_terminal_server::dispatch::lanes::restart::{RestartLane, ServerRestart};
use ovis_terminal_server::dispatch::lanes::update::UpdateLane;
use ovis_terminal_server::dispatch::lanes::update::control::ControlRestart;
use ovis_terminal_services::update::{Config, CurrentBuild, Requirement, Updates};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::handler::Dispatch;
use super::serve::at;
use crate::error::RunnerError;

/// The unadvertised probe reporting release digests and escaped files.
pub(crate) const PROBE: &str = "fixture.update";

const SPAWN_WAIT: Duration = Duration::from_secs(15);

/// The holder whose program the update lane must never end.
#[derive(Debug)]
pub(crate) struct Held(Holder);

impl Held {
    /// Ends the holder's workers once the dispatcher is gone.
    pub(crate) fn shutdown(self) -> Result<(), RunnerError> {
        self.0
            .shutdown()
            .map(drop)
            .map_err(at("shut the update holder down"))
    }
}

fn script(version: &str, status: u8) -> String {
    let report = json!({"binary": "ovis-terminal", "version": version, "generation": "1"});
    format!("#!/bin/sh\nprintf '%s\\n' '{report}'\nexit {status}\n")
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect()
}

/// Archives `members` of `dir` as ustar into the release source; returns the digest.
fn publish(
    root: &Path,
    (version, dir): (&str, &Path),
    members: &[&str],
) -> Result<String, RunnerError> {
    let target = BuildTarget::host();
    let out = root.join("releases").join(version);
    std::fs::create_dir_all(&out).map_err(at("create the release source"))?;
    let archive = out.join(format!("ovis-terminal-{version}-{target}.tar"));
    let status = Command::new("tar")
        .args(["--format=ustar", "-P", "-cf"])
        .arg(&archive)
        .arg("-C")
        .arg(dir)
        .args(members)
        .status()
        .map_err(at("run tar"))?;
    if !status.success() {
        return Err(at("archive a release")(format!("tar exited {status}")));
    }
    Ok(hex(&std::fs::read(&archive).map_err(at("read a release"))?))
}

fn build(root: &Path, version: &str, status: u8) -> Result<PathBuf, RunnerError> {
    let dir = root.join("work").join(version);
    std::fs::create_dir_all(&dir).map_err(at("create a build"))?;
    let manifest = json!({"version": version, "target": BuildTarget::host().as_str(),
        "protocol_generations": ["1"], "holder_generations": ["2"]});
    std::fs::write(dir.join("build.json"), manifest.to_string()).map_err(at("write build.json"))?;
    std::fs::write(dir.join("ovis-terminal"), script(version, status))
        .map_err(at("write a build"))?;
    Ok(dir)
}

fn releases(root: &Path) -> Result<Value, RunnerError> {
    let ready = publish(
        root,
        ("2.0.0", &build(root, "2.0.0", 0)?),
        &["build.json", "ovis-terminal"],
    )?;
    let broken = publish(
        root,
        ("3.0.0", &build(root, "3.0.0", 1)?),
        &["build.json", "ovis-terminal"],
    )?;
    std::fs::write(root.join("work").join("escape"), "outside").map_err(at("write the escape"))?;
    let climbing = publish(
        root,
        ("4.0.0", &build(root, "4.0.0", 0)?),
        &["build.json", "../escape"],
    )?;
    Ok(json!({"target": BuildTarget::host().as_str(),
        "releases": {"ready": ready, "broken": broken, "climbing": climbing}}))
}

fn control(
    root: &Path,
    (authority, bearer): (&Arc<Authority<RegistryIdentities>>, &str),
    runtime: RuntimeIdentity,
) -> Result<(Control, Holder), RunnerError> {
    let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1000)))
        .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))
        .map_err(at("admit the runtime session"))?;
    let session = authority
        .authenticate(&admission, Presentation::new(bearer, TimestampMs::new(1)))
        .map_err(at("authenticate the runtime session"))?;
    let holder = Holder::new(HolderConfig::new(
        HolderDescriptor::new(
            HolderBuildId::new("conformance-update".into()),
            HolderInstanceId::new(uuid::Uuid::now_v7().to_string()),
        ),
        HolderLimits::DEFAULT,
    ));
    let state = root.join("control");
    std::fs::create_dir_all(&state).map_err(at("create the control state"))?;
    let plan = ControlPlan::new(state, (holder.clone(), Arc::clone(authority)), session);
    let (control, _) = Control::open(plan, runtime).map_err(at("open the control"))?;
    let program = Program::new(
        vec!["/bin/sh".into(), "-c".into(), "read gate".into()],
        root.to_path_buf(),
        [("PATH".into(), "/usr/bin:/bin".into())].into(),
    )
    .map_err(at("describe the held program"))?;
    let geometry =
        serde_json::from_value(json!({"cols": 80, "rows": 24})).map_err(at("geometry"))?;
    let launch = Launch::new(program, geometry).map_err(at("describe the launch"))?;
    control
        .with_terminals(|terminals, session| {
            let call = Call::new(session, OpId::new(uuid::Uuid::now_v7().to_string()));
            let admitted = terminals.run(call, launch)?;
            terminals.await_spawn(admitted.id(), SPAWN_WAIT)
        })
        .map_err(at("spawn the held program"))?;
    Ok((control, holder))
}

/// Builds the update lane over a fresh install root below `root/update`; it blocks on
/// file and SQLite I/O, so the caller runs it on the blocking pool.
fn prepare(
    root: &Path,
    (authority, bearer): (&Arc<Authority<RegistryIdentities>>, &str),
    runtime: RuntimeIdentity,
) -> Result<(UpdateLane, Arc<Control>, Held), RunnerError> {
    let root = root.join("update");
    std::fs::create_dir_all(&root).map_err(at("create the update root"))?;
    let facts = releases(&root)?;
    std::fs::write(root.join("releases.json"), facts.to_string()).map_err(at("record releases"))?;
    let current = root.join("ovis-terminal-1.0.0");
    std::fs::write(&current, script("1.0.0", 0)).map_err(at("write the current build"))?;
    let (control, holder) = control(&root, (authority, bearer), runtime)?;
    let config = Config::new(
        (root.join("install"), root.join("releases")),
        CurrentBuild::new("1.0.0".parse().map_err(at("parse the version"))?, current),
        Requirement::new(ProtocolGeneration::new(1), HolderGeneration::new(2)),
    );
    let control = Arc::new(control);
    let restart = Arc::new(ControlRestart::new(Arc::clone(&control)));
    let service = Updates::open(config, restart).map_err(at("open the install root"))?;
    Ok((
        UpdateLane::new(Arc::new(service), runtime.machine_id()),
        control,
        Held(holder),
    ))
}

/// Prepares the lane on the blocking pool and registers it on the fixture's dispatcher.
pub(crate) async fn register(
    dispatch: &mut Dispatch,
    (root, authority, bearer): (&Path, &Arc<Authority<RegistryIdentities>>, &str),
    runtime: RuntimeIdentity,
) -> Result<Held, RunnerError> {
    let (root, authority, bearer) = (root.to_path_buf(), Arc::clone(authority), bearer.to_owned());
    let (lane, control, held) =
        tokio::task::spawn_blocking(move || prepare(&root, (&authority, &bearer), runtime))
            .await
            .map_err(at("join the update fixture setup"))??;
    lane.register(dispatch).map_err(at("register update.*"))?;
    let restart = RestartLane::new(control, runtime.machine_id());
    dispatch
        .register(ServerRestart::new(&restart))
        .map_err(at("register server.restart"))?;
    Ok(held)
}

fn escaped(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            let path = entry.path();
            path.file_name().is_some_and(|name| name == "escape") || escaped(&path)
        })
}

/// `fixture.update`: the published digests and whether a file escaped into the install root.
pub(crate) fn probe(root: &Path) -> Result<Value, BoundaryError> {
    let root = root.join("update");
    let unreadable = || {
        BoundaryError::domain(ErrorKind::from("internal".to_owned()), false)
            .with_hint("the fixture lost its release record")
    };
    let bytes = std::fs::read(root.join("releases.json")).map_err(|_| unreadable())?;
    let mut facts: Value = serde_json::from_slice(&bytes).map_err(|_| unreadable())?;
    if let Some(object) = facts.as_object_mut() {
        object.insert("escaped".to_owned(), json!(escaped(&root.join("install"))));
    }
    Ok(facts)
}

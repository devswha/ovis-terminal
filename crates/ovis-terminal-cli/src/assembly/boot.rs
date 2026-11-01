//! The pieces one server lifetime owns: identity, authority and its two bearers, the
//! stores, the shared holder and the terminal lifecycle over it.

use std::path::Path;
use std::sync::Arc;

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, FileEvidence, Grant, Issuance, IssueRequest,
    LocalAccount, OwnerSpec, PeerEvidence, Permission, Presentation, PrincipalId, RemoteTransport,
    ResourceScope, RuntimeBinding, ScopeResource, Session, UnixSocketEvidence, UnixUid,
};
use ovis_terminal_core::events::{Coordinator, EventError, Filter, SnapshotProducer};
use ovis_terminal_core::geometry::GeometryEvent;
use ovis_terminal_core::leases::LeaseEvent;
use ovis_terminal_core::registry::{MachineId, RegistryIdentities, RuntimeIdentity};
use ovis_terminal_core::restart::{Boot, Control, ControlPlan};
use ovis_terminal_holder::protocol::{HolderBuildId, HolderDescriptor, HolderInstanceId};
use ovis_terminal_holder::unix::{Holder, HolderConfig, HolderLimits};
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_store::{Snapshot, Store, StoreOptions};

use super::AssemblyError;

/// Attach-side event logs carry no snapshot sections of their own.
#[derive(Debug)]
pub(super) struct NoSections;

impl SnapshotProducer for NoSections {
    type Sections = ();

    fn capture(&self, _snapshot: &Snapshot, _filter: &Filter) -> Result<(), EventError> {
        Ok(())
    }
}

/// Committed lease transitions of every attach.
pub(super) type LeaseLog = Coordinator<LeaseEvent, NoSections>;
/// Committed geometry transitions of every terminal.
pub(super) type GeometryLog = Coordinator<GeometryEvent, NoSections>;

/// The owner bearer, an observe-only bearer and a writer bearer (observe, type, answer
/// prompts), printed once in the ready line.
#[derive(Debug)]
pub(super) struct Bearers {
    pub(super) owner: String,
    pub(super) observer: String,
    pub(super) writer: String,
}

/// The owner session the restart control runs its own steps under: the in-process peer
/// is the server's account, evidenced by the private state directory it owns.
fn owner_session(
    authority: &Authority<RegistryIdentities>,
    (state, bearer): (&Path, &str),
) -> Result<Session, AssemblyError> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata =
        std::fs::metadata(state).map_err(AssemblyError::start("inspect the state directory"))?;
    let uid = UnixUid::new(rustix::process::getuid().as_raw());
    let evidence = FileEvidence::new(UnixUid::new(metadata.uid()), metadata.mode() & 0o7777);
    let admission = EndpointPolicy::new(LocalAccount::Unix(uid))
        .admit(&PeerEvidence::UnixSocket(UnixSocketEvidence::new(
            evidence, evidence, uid,
        )))
        .map_err(AssemblyError::start("admit the control session"))?;
    authority
        .authenticate(&admission, Presentation::new(bearer, now()?))
        .map_err(AssemblyError::start("authenticate the control session"))
}

fn now() -> Result<TimestampMs, AssemblyError> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(AssemblyError::start("read the clock"))?;
    u64::try_from(elapsed.as_millis())
        .map(TimestampMs::new)
        .map_err(AssemblyError::start("read the clock"))
}

/// Everything started before the endpoint is bound.
#[derive(Debug)]
pub(super) struct Booted {
    pub(super) identity: RuntimeIdentity,
    pub(super) authority: Arc<Authority<RegistryIdentities>>,
    pub(super) bearers: Bearers,
    pub(super) holder: Holder,
    pub(super) control: Arc<Control>,
    pub(super) boot: Boot,
    pub(super) inherited: Vec<ovis_terminal_core::recovery::Recorded>,
    pub(super) recorded: Vec<ovis_terminal_core::terminals::Record>,
    pub(super) leases: LeaseLog,
    pub(super) geometry: GeometryLog,
}

/// Issues a bearer holding exactly `grants` on the machine, authorized by the owner bearer.
fn delegate(
    authority: &Authority<RegistryIdentities>,
    (owner, machine): (&str, MachineId),
    (label, grants): (&str, &[Permission]),
) -> Result<String, AssemblyError> {
    let now = TimestampMs::new(1);
    let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(
        rustix::process::getuid().as_raw(),
    )))
    .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))
    .map_err(AssemblyError::start("admit the issuing owner"))?;
    let owner = authority
        .authenticate(&admission, Presentation::new(owner, now))
        .map_err(AssemblyError::start("authenticate the issuing owner"))?;
    let principal =
        PrincipalId::parse(label).map_err(AssemblyError::start("delegate principal"))?;
    let grants = grants
        .iter()
        .map(|permission| {
            Grant::new(
                *permission,
                ResourceScope::new(machine, ScopeResource::Machine),
            )
        })
        .collect();
    let request = IssueRequest::new(principal, grants, TimestampMs::new(u64::MAX));
    let issued = authority
        .issue(&owner, Issuance::new(request, now))
        .map_err(AssemblyError::start("issue a delegate bearer"))?;
    Ok(issued.bearer().to_owned())
}

/// Opens the store `name` below `state`.
pub(super) fn store(state: &Path, name: &str) -> Result<Store, AssemblyError> {
    Store::open(&state.join(name), StoreOptions::default())
        .map_err(AssemblyError::start("open a store"))
}

/// Starts every piece of one server lifetime under `state`, off the async runtime
/// because adopting the holder and opening the stores block on their writers.
pub(super) async fn boot(state: &Path) -> Result<Booted, AssemblyError> {
    let state = state.to_path_buf();
    tokio::task::spawn_blocking(move || boot_blocking(&state))
        .await
        .map_err(AssemblyError::start("boot the server"))?
}

fn boot_blocking(state: &Path) -> Result<Booted, AssemblyError> {
    let identity = super::identity::next_boot(state)?;
    let inherited = super::recovery::inherited(state, identity)?;
    let binding = RuntimeBinding::new(identity.machine_id(), identity.runtime_id());
    let owner = PrincipalId::parse("owner").map_err(AssemblyError::start("owner principal"))?;
    let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(
        binding,
        OwnerSpec::new(owner, TimestampMs::new(u64::MAX)),
    ))
    .map_err(AssemblyError::start("bootstrap the authority"))?;
    let authority = Arc::new(authority);
    let bearers = Bearers {
        observer: delegate(
            &authority,
            (issued.bearer(), identity.machine_id()),
            ("observer", &[Permission::Observe]),
        )?,
        writer: delegate(
            &authority,
            (issued.bearer(), identity.machine_id()),
            (
                "writer",
                &[
                    Permission::Observe,
                    Permission::Interact,
                    Permission::PromptAnswer,
                ],
            ),
        )?,
        owner: issued.bearer().to_owned(),
    };
    let descriptor = HolderDescriptor::new(
        HolderBuildId::new(crate::VERSION.to_owned()),
        HolderInstanceId::new(uuid::Uuid::now_v7().to_string()),
    );
    let holder = Holder::new(HolderConfig::new(descriptor, HolderLimits::DEFAULT));
    let plan = ControlPlan::new(
        state.join("control"),
        (holder.clone(), Arc::clone(&authority)),
        owner_session(&authority, (state, &bearers.owner))?,
    );
    std::fs::create_dir_all(state.join("control"))
        .map_err(AssemblyError::start("create the control state"))?;
    let (control, boot) =
        Control::open(plan, identity).map_err(AssemblyError::start("adopt the holder"))?;
    let identity = control
        .identity()
        .map_err(AssemblyError::start("read the boot identity"))?;
    let recorded = control
        .with_terminals(|terminals, session| terminals.list(session))
        .map_err(AssemblyError::start("read the recorded terminals"))?;
    let leases = Coordinator::new(store(state, "leases.db")?, identity, NoSections);
    let geometry = Coordinator::new(store(state, "geometry.db")?, identity, NoSections);
    Ok(Booted {
        identity,
        authority,
        bearers,
        holder,
        control: Arc::new(control),
        boot,
        inherited,
        recorded,
        leases,
        geometry,
    })
}

//! The pieces one server lifetime owns: identity, authority and its two bearers, the
//! stores, the shared holder and the terminal lifecycle over it.

use std::path::Path;
use std::sync::Arc;

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, Grant, Issuance, IssueRequest, LocalAccount,
    OwnerSpec, PeerEvidence, Permission, Presentation, PrincipalId, RemoteTransport, ResourceScope,
    RuntimeBinding, ScopeResource, UnixUid,
};
use ovis_terminal_core::events::{Coordinator, EventError, Filter, SnapshotProducer};
use ovis_terminal_core::geometry::GeometryEvent;
use ovis_terminal_core::leases::LeaseEvent;
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RegistryIdentities, RuntimeId, RuntimeIdentity,
    ServerBootId,
};
use ovis_terminal_core::terminals::{Host, Terminals};
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

/// Everything started before the endpoint is bound.
#[derive(Debug)]
pub(super) struct Booted {
    pub(super) identity: RuntimeIdentity,
    pub(super) authority: Arc<Authority<RegistryIdentities>>,
    pub(super) bearers: Bearers,
    pub(super) holder: Holder,
    pub(super) terminals: Terminals,
    pub(super) leases: Arc<LeaseLog>,
    pub(super) geometry: GeometryLog,
}

fn identity() -> Result<RuntimeIdentity, AssemblyError> {
    let machine: MachineId = format!("machine:{}", uuid::Uuid::now_v7())
        .parse()
        .map_err(AssemblyError::start("mint the machine identity"))?;
    let host = HostBootId::try_from(1).map_err(AssemblyError::start("host boot identity"))?;
    let runtime = RuntimeId::try_from(1).map_err(AssemblyError::start("runtime identity"))?;
    let boot = ServerBootId::try_from(1).map_err(AssemblyError::start("server boot identity"))?;
    Ok(RuntimeIdentity::new(
        HostIdentity::new(machine, host),
        runtime,
        boot,
    ))
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

fn store(state: &Path, name: &str) -> Result<Store, AssemblyError> {
    Store::open(&state.join(name), StoreOptions::default())
        .map_err(AssemblyError::start("open a store"))
}

/// Starts every piece of one server lifetime under `state`.
pub(super) fn boot(state: &Path) -> Result<Booted, AssemblyError> {
    let identity = identity()?;
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
    let host = Host::new(identity, Arc::clone(&authority), holder.clone());
    let terminals = Terminals::new(store(state, "terminals.db")?, host)
        .map_err(AssemblyError::start("adopt the holder"))?;
    let leases = Arc::new(Coordinator::new(
        store(state, "leases.db")?,
        identity,
        NoSections,
    ));
    let geometry = Coordinator::new(store(state, "geometry.db")?, identity, NoSections);
    Ok(Booted {
        identity,
        authority,
        bearers,
        holder,
        terminals,
        leases,
        geometry,
    })
}

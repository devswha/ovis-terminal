//! `attach.close` revokes the closing owner's leases before it answers.
//!
//! The hub is driven directly with no pump task, so the lease table seen right after
//! `close` returns is exactly what a write admitted next would be checked against.

use std::num::NonZeroU64;
use std::sync::Arc;

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, FileEvidence, LocalAccount, OwnerSpec,
    PeerEvidence, Presentation, PrincipalId, ResolvedTarget, RuntimeBinding, TargetResource,
    UnixSocketEvidence, UnixUid,
};
use ovis_terminal_core::leases::{Caller, Owner, TerminalLeases, Transition};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, LeaseId, MachineId, RegistryIdentities, RuntimeId, RuntimeIdentity,
    ServerBootId, TerminalGeneration, TerminalId,
};
use ovis_terminal_core::screen::{Screen, ScreenBounds, ScreenConfig};
use ovis_terminal_holder::protocol::{
    ProcessIncarnation, TerminalId as HolderTerminalId, TerminalIncarnation, TerminalTarget,
};
use ovis_terminal_protocol::schema::ids::{ClientId, StreamId};
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_protocol::schema::records::LeaseRole;
use ovis_terminal_protocol::schema::scalar::{EventSeq, TimestampMs};
use ovis_terminal_store::OpId;
use ovis_terminal_vt::{Geometry, HistoryLimit, VtConfig};

use super::super::error::StreamError;
use super::super::hub::{StreamSetup, TerminalStreams};
use super::super::journal::LeaseJournal;
use super::super::limits::{SessionBacklog, StreamLimits};
use super::super::open::OpenRequest;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Lease transitions are accepted and dropped; the test reads the table, not events.
#[derive(Debug)]
struct Discard;

impl LeaseJournal for Discard {
    fn record(&self, _op: OpId, _transition: Transition) -> Result<Option<EventSeq>, StreamError> {
        Ok(None)
    }
}

fn hub() -> TestResult<(
    Arc<TerminalStreams>,
    Arc<Authority<RegistryIdentities>>,
    String,
)> {
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000064".parse()?;
    let runtime = RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(7)?,
        ServerBootId::try_from(1)?,
    );
    let binding = RuntimeBinding::new(runtime.machine_id(), runtime.runtime_id());
    let owner = OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX));
    let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner))?;
    let authority = Arc::new(authority);
    let terminal = TerminalId::try_from(7)?;
    let target = ResolvedTarget::new(
        runtime.machine_id(),
        TargetResource::Terminal {
            placement: None,
            terminal,
            incarnation: TerminalGeneration::new(1)?,
        },
    );
    let leases = TerminalLeases::new(Arc::clone(&authority), target)?;
    let holder = TerminalTarget {
        terminal_id: HolderTerminalId::new("terminal:7".to_owned()),
        terminal_incarnation: TerminalIncarnation::new(1),
        process_incarnation: Some(ProcessIncarnation::new(1)),
    };
    let vt = VtConfig::new(Geometry::new(40, 8)?, HistoryLimit::new(200));
    let screen = Screen::new(holder, ScreenConfig::new(vt, ScreenBounds::DEFAULT))?;
    let setup = StreamSetup::new(
        StreamLimits::advertised(&Limits::v1()),
        Arc::new(SessionBacklog::new(64 * 1024 * 1024)),
        Arc::new(Discard),
    );
    Ok((
        TerminalStreams::new(screen, leases, setup),
        authority,
        issued.bearer().to_owned(),
    ))
}

#[test]
fn close_revokes_the_interact_lease_before_it_returns() -> TestResult {
    let (hub, authority, bearer) = hub()?;
    let uid = UnixUid::new(1000);
    let evidence = UnixSocketEvidence::new(
        FileEvidence::new(uid, 0o600),
        FileEvidence::new(uid, 0o700),
        uid,
    );
    let admission =
        EndpointPolicy::new(LocalAccount::Unix(uid)).admit(&PeerEvidence::UnixSocket(evidence))?;
    let at = TimestampMs::new(1_000);
    let session = authority.authenticate(&admission, Presentation::new(&bearer, at))?;
    let client = ClientId::new("desk".to_owned());
    let owner = Owner::new(
        session.principal().clone(),
        client.clone(),
        session.connection(),
    );
    let stream = StreamId::new("stream:desk".to_owned());
    let request = OpenRequest {
        caller: Caller::new(&session, &client, at),
        owner: &owner,
        role: LeaseRole::Interact,
        takeover: None,
        fresh: LeaseId::new(NonZeroU64::MIN),
        stream: &stream,
    };
    hub.open(request, OpId::new("op-open"))?;
    let held = hub
        .lock()?
        .leases
        .view()
        .interact
        .map(|lease| lease.lease_id);
    assert_eq!(
        held,
        Some(LeaseId::new(NonZeroU64::MIN)),
        "the open granted interact"
    );

    hub.close(&stream)?;

    let after = hub
        .lock()?
        .leases
        .view()
        .interact
        .map(|lease| lease.lease_id);
    assert_eq!(
        after, None,
        "close answered with the interact lease still held"
    );
    Ok(())
}

//! A real authority, lease table, VT modes and wait hub for one launched terminal.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ovis_terminal_authz::{
    AccessRequest, Action, Authority, AuthorityConfig, CredentialId, Decision, EndpointPolicy,
    FileEvidence, Grant, Issuance, IssueRequest, LocalAccount, OwnerSpec, PeerAdmission,
    PeerEvidence, Permission, Presentation, PrincipalId, ResolvedTarget, ResourceScope, Revocation,
    RuntimeBinding, ScopeResource, Session, TargetResource, UnixSocketEvidence, UnixUid,
};
use ovis_terminal_holder::protocol::{
    ProcessIncarnation, TerminalId as HolderTerminalId, TerminalIncarnation, TerminalTarget,
};
use ovis_terminal_protocol::domain::template_launch::FirstMessageRecord;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::TerminalId as WireTerminalId;
use ovis_terminal_protocol::schema::scalar::{Incarnation, TimestampMs};
use ovis_terminal_vt::{Geometry, HistoryLimit, Vt, VtConfig};
use uuid::Uuid;

use crate::input::encoding::ModeSnapshot;
use crate::launch::QueuedMessage;
use crate::leases::TerminalLeases;
use crate::registry::{MachineId, RegistryIdentities, RuntimeId, TerminalGeneration, TerminalId};
use crate::screen::{Screen, ScreenBounds, ScreenConfig};
use crate::waits::Waits;

pub(super) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Registry = RegistryIdentities;

/// Request instant of every fixture call.
pub(super) const NOW: TimestampMs = TimestampMs::new(1_000);
/// Bound on every wait a test expects a feed to resolve.
pub(super) const BOUND: Duration = Duration::from_secs(20);

pub(super) fn bound() -> TestResult<Instant> {
    Ok(Instant::now().checked_add(BOUND).ok_or("clock overflow")?)
}

/// The runtime owner and one launching principal with input rights on the machine.
pub(super) struct Fixture {
    pub(super) authority: Arc<Authority<Registry>>,
    pub(super) owner: Session,
    pub(super) launcher: Session,
    launcher_credential: CredentialId,
    machine: MachineId,
}

impl Fixture {
    pub(super) fn new() -> TestResult<Self> {
        let machine = MachineId::new(Uuid::from_u128(35))?;
        let binding = RuntimeBinding::<Registry>::new(machine, RuntimeId::try_from(1)?);
        let spec = OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(9_000));
        let (authority, bootstrap) = Authority::bootstrap(AuthorityConfig::new(binding, spec))?;
        let admission = admission()?;
        let owner =
            authority.authenticate(&admission, Presentation::new(bootstrap.bearer(), NOW))?;
        let scope = ResourceScope::new(machine, ScopeResource::Machine);
        let grants = [Permission::Observe, Permission::Interact]
            .into_iter()
            .map(|permission| Grant::new(permission, scope.clone()))
            .collect();
        let request = IssueRequest::new(
            PrincipalId::parse("launcher")?,
            grants,
            TimestampMs::new(8_000),
        );
        let issued = authority.issue(&owner, Issuance::new(request, NOW))?;
        let launcher =
            authority.authenticate(&admission, Presentation::new(issued.bearer(), NOW))?;
        Ok(Self {
            authority: Arc::new(authority),
            owner,
            launcher,
            launcher_credential: *issued.id(),
            machine,
        })
    }

    /// The launcher's input decision on the machine, as taken at launch admission.
    pub(super) fn decision(&self) -> TestResult<Decision<Registry>> {
        let target = ResolvedTarget::new(self.machine, TargetResource::Machine);
        Ok(self.authority.authorize(
            &self.launcher,
            &AccessRequest::new(Action::Input, target, NOW),
        )?)
    }

    pub(super) fn revoke_launcher(&self) -> TestResult {
        self.authority
            .revoke(&self.owner, &Revocation::new(self.launcher_credential, NOW))?;
        Ok(())
    }

    pub(super) fn leases(&self) -> TestResult<TerminalLeases> {
        let terminal = TargetResource::Terminal {
            placement: None,
            terminal: TerminalId::try_from(100)?,
            incarnation: TerminalGeneration::new(1)?,
        };
        Ok(TerminalLeases::new(
            Arc::clone(&self.authority),
            ResolvedTarget::new(self.machine, terminal),
        )?)
    }
}

fn admission() -> TestResult<PeerAdmission> {
    let uid = UnixUid::new(1000);
    let socket = UnixSocketEvidence::new(
        FileEvidence::new(uid, 0o600),
        FileEvidence::new(uid, 0o700),
        uid,
    );
    Ok(EndpointPolicy::new(LocalAccount::Unix(uid)).admit(&PeerEvidence::UnixSocket(socket))?)
}

/// Modes of a fresh emulator after `output`.
pub(super) fn modes(output: &[u8]) -> TestResult<ModeSnapshot> {
    let mut vt = Vt::new(VtConfig::new(Geometry::new(80, 24)?, HistoryLimit::new(10)));
    vt.feed(output);
    Ok(ModeSnapshot::capture(&vt))
}

/// A wait hub over a fresh screen of terminal 100, process 1.
pub(super) fn hub() -> TestResult<Waits> {
    let target = TerminalTarget {
        terminal_id: HolderTerminalId::new("100".into()),
        terminal_incarnation: TerminalIncarnation::new(1),
        process_incarnation: Some(ProcessIncarnation::new(1)),
    };
    let slice = NonZeroUsize::new(1024).ok_or("positive")?;
    let bounds = ScreenBounds::DEFAULT
        .with_work_budget(slice)
        .with_checkpoint_interval(slice);
    let config = ScreenConfig::new(
        VtConfig::new(Geometry::new(80, 24)?, HistoryLimit::new(10)),
        bounds,
    );
    Ok(Waits::new(Screen::new(target, config)?))
}

/// A message queued by `principal` under a fresh launch operation.
pub(super) fn queued(principal: &str, text: &str) -> TestResult<QueuedMessage> {
    let record = FirstMessageRecord::accepted(
        (
            OpId::new(Uuid::now_v7().to_string()),
            WireTerminalId::new("100".to_owned()),
        ),
        Incarnation::new(1),
        TimestampMs::new(31_000),
    );
    Ok(QueuedMessage::new(
        PrincipalId::parse(principal)?,
        text.to_owned(),
        record,
    ))
}

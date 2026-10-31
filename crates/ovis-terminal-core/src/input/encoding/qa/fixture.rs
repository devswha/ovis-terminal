//! Real VT mode state and a real authority deciding the raw-input grant.

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, FileEvidence, Grant, Issuance, IssueRequest,
    LocalAccount, OwnerSpec, PeerAdmission, PeerEvidence, Permission, Presentation, PrincipalId,
    ResolvedTarget, ResourceScope, RuntimeBinding, ScopeResource, Session, TargetResource,
    UnixSocketEvidence, UnixUid,
};
use ovis_terminal_core::input::encoding::{EncodingError, RawGrant, RawRequest};
use ovis_terminal_core::registry::{
    MachineId, RegistryIdentities, RuntimeId, TerminalGeneration, TerminalId,
};
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_vt::{Geometry, HistoryLimit, Vt, VtConfig};
use uuid::Uuid;

use super::TestResult;

/// Request time of every fixture call.
pub const NOW: TimestampMs = TimestampMs::new(1_000);
/// Interact plus the separate raw-input grant.
pub const OPERATOR: &[Permission] = &[
    Permission::Observe,
    Permission::Interact,
    Permission::RawInput,
];
/// Ordinary input without the raw-input grant.
pub const PLAIN_INTERACTOR: &[Permission] = &[Permission::Observe, Permission::Interact];

/// An 80x24 emulator that already consumed `output`, so its modes are the program's own.
///
/// # Errors
///
/// Fails when the geometry is rejected.
pub fn vt(output: &[u8]) -> TestResult<Vt> {
    let mut vt = Vt::new(VtConfig::new(
        Geometry::new(80, 24)?,
        HistoryLimit::new(100),
    ));
    vt.feed(output);
    Ok(vt)
}

/// A bootstrapped authority and one terminal target.
#[derive(Debug)]
pub struct Fixture {
    authority: Authority<RegistryIdentities>,
    target: ResolvedTarget<RegistryIdentities>,
    admission: PeerAdmission,
    owner: Session,
}

impl Fixture {
    /// Bootstraps the authority.
    ///
    /// # Errors
    ///
    /// Fails when the authority refuses the setup step.
    pub fn new() -> TestResult<Self> {
        let machine = MachineId::new(Uuid::from_u128(32))?;
        let binding = RuntimeBinding::<RegistryIdentities>::new(machine, RuntimeId::try_from(1)?);
        let spec = OwnerSpec::new(
            PrincipalId::parse("runtime-owner")?,
            TimestampMs::new(9_000),
        );
        let (authority, bootstrap) = Authority::bootstrap(AuthorityConfig::new(binding, spec))?;
        let uid = UnixUid::new(1000);
        let socket = UnixSocketEvidence::new(
            FileEvidence::new(uid, 0o600),
            FileEvidence::new(uid, 0o700),
            uid,
        );
        let admission = EndpointPolicy::new(LocalAccount::Unix(uid))
            .admit(&PeerEvidence::UnixSocket(socket))?;
        let owner =
            authority.authenticate(&admission, Presentation::new(bootstrap.bearer(), NOW))?;
        let terminal = TargetResource::Terminal {
            placement: None,
            terminal: TerminalId::try_from(32)?,
            incarnation: TerminalGeneration::new(1)?,
        };
        Ok(Self {
            authority,
            target: ResolvedTarget::new(machine, terminal),
            admission,
            owner,
        })
    }

    /// Issues `permissions` on the fixture terminal to a fresh principal and authenticates it.
    ///
    /// # Errors
    ///
    /// Fails when the authority refuses the setup step.
    pub fn session(&self, permissions: &[Permission]) -> TestResult<Session> {
        let TargetResource::Terminal { terminal, .. } = self.target.resource() else {
            return Err("fixture target is not a terminal".into());
        };
        let scope = ResourceScope::new(
            *self.target.machine(),
            ScopeResource::Terminal { id: *terminal },
        );
        let grants = permissions
            .iter()
            .map(|permission| Grant::new(*permission, scope.clone()))
            .collect();
        let label = PrincipalId::parse(&format!("client-{}", Uuid::now_v7().simple()))?;
        let request = IssueRequest::new(label, grants, TimestampMs::new(8_000));
        let issued = self
            .authority
            .issue(&self.owner, Issuance::new(request, NOW))?;
        Ok(self
            .authority
            .authenticate(&self.admission, Presentation::new(issued.bearer(), NOW))?)
    }

    /// Asks the authority for the raw-input grant on the fixture terminal.
    ///
    /// # Errors
    ///
    /// Returns `raw_not_authorized` when the session lacks the raw-input grant.
    pub fn raw_grant(&self, session: &Session) -> Result<RawGrant, EncodingError> {
        RawGrant::decide(
            &self.authority,
            session,
            RawRequest::new(self.target.clone(), NOW),
        )
    }

    /// The authority, for write-boundary rechecks.
    pub const fn authority(&self) -> &Authority<RegistryIdentities> {
        &self.authority
    }
}

//! Real authority, authenticated sessions and one terminal target for the lease scenarios.

use std::path::PathBuf;
use std::sync::Arc;

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, FileEvidence, Grant, Issuance, IssueRequest,
    IssuedCredential, LocalAccount, OwnerSpec, PeerAdmission, PeerEvidence, Permission,
    Presentation, PrincipalId, ResolvedTarget, ResourceScope, RuntimeBinding, ScopeResource,
    Session, TargetResource, UnixSocketEvidence, UnixUid,
};
use ovis_terminal_core::events::{EventError, Filter, SnapshotProducer};
use ovis_terminal_core::leases::{Caller, ControlSlot, Owner, Takeover, TerminalLeases};
use ovis_terminal_core::registry::{
    LeaseGeneration, LeaseId, MachineId, RegistryIdentities, RuntimeId, TerminalGeneration,
    TerminalId,
};
use ovis_terminal_protocol::schema::ids::ClientId;
use ovis_terminal_protocol::schema::methods::attach::TakeoverRequest;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_store::Snapshot;
use serde_json::json;
use uuid::Uuid;

use super::TestResult;

/// Request time of every fixture call.
pub const NOW: TimestampMs = TimestampMs::new(1_000);
/// Observe-only grant.
pub const OBSERVER: &[Permission] = &[Permission::Observe];
/// Observe and interact grants.
pub const INTERACTOR: &[Permission] = &[Permission::Observe, Permission::Interact];
/// Interact plus the separate takeover grant.
pub const TAKEOVER: &[Permission] = &[
    Permission::Observe,
    Permission::Interact,
    Permission::Takeover,
];

/// One authenticated principal attached as the client label of the same name.
#[derive(Debug)]
#[non_exhaustive]
pub struct Client {
    /// Authenticated session.
    pub session: Session,
    /// Client label.
    pub client: ClientId,
}

impl Client {
    /// The caller this client acts as.
    pub const fn caller(&self) -> Caller<'_> {
        Caller::new(&self.session, &self.client, NOW)
    }

    /// The owner its leases carry.
    pub fn owner(&self) -> Owner {
        Owner::new(
            self.session.principal().clone(),
            self.client.clone(),
            self.session.connection(),
        )
    }
}

/// A bootstrapped authority and one terminal target.
#[derive(Debug)]
pub struct Fixture {
    authority: Arc<Authority<RegistryIdentities>>,
    target: ResolvedTarget<RegistryIdentities>,
    admission: PeerAdmission,
    owner: Session,
    next_lease: u64,
}

impl Fixture {
    /// Bootstraps the authority.
    ///
    /// # Errors
    ///
    /// Fails when the authority refuses the setup step.
    pub fn new() -> TestResult<Self> {
        let machine = MachineId::new(Uuid::from_u128(24))?;
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
            terminal: TerminalId::try_from(7)?,
            incarnation: TerminalGeneration::new(1)?,
        };
        Ok(Self {
            authority: Arc::new(authority),
            target: ResolvedTarget::new(machine, terminal),
            admission,
            owner,
            next_lease: 0,
        })
    }

    /// A fresh lease table on the fixture terminal.
    ///
    /// # Errors
    ///
    /// Fails when the authority refuses the setup step.
    pub fn leases(&self) -> TestResult<TerminalLeases> {
        Ok(TerminalLeases::new(
            Arc::clone(&self.authority),
            self.target.clone(),
        )?)
    }

    /// Issues `permissions` on the terminal to a principal named `label` and authenticates it.
    ///
    /// # Errors
    ///
    /// Fails when the authority refuses the setup step.
    pub fn client(&self, label: &str, permissions: &[Permission]) -> TestResult<Client> {
        let issued = self.issue(label, permissions)?;
        self.connect(&issued, label)
    }

    /// Two connections of one principal named `label`, both attached under that label.
    ///
    /// # Errors
    ///
    /// Fails when the authority refuses the setup step.
    pub fn twins(&self, label: &str, permissions: &[Permission]) -> TestResult<(Client, Client)> {
        let issued = self.issue(label, permissions)?;
        Ok((self.connect(&issued, label)?, self.connect(&issued, label)?))
    }

    fn connect(
        &self,
        issued: &IssuedCredential<RegistryIdentities>,
        label: &str,
    ) -> TestResult<Client> {
        let session = self
            .authority
            .authenticate(&self.admission, Presentation::new(issued.bearer(), NOW))?;
        Ok(Client {
            session,
            client: ClientId::new(label.to_owned()),
        })
    }

    fn issue(
        &self,
        label: &str,
        permissions: &[Permission],
    ) -> TestResult<IssuedCredential<RegistryIdentities>> {
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
        let request =
            IssueRequest::new(PrincipalId::parse(label)?, grants, TimestampMs::new(8_000));
        Ok(self
            .authority
            .issue(&self.owner, Issuance::new(request, NOW))?)
    }

    /// A lease identity never handed out before by this fixture.
    ///
    /// # Errors
    ///
    /// Fails when the authority refuses the setup step.
    pub fn fresh(&mut self) -> TestResult<LeaseId> {
        self.next_lease = self
            .next_lease
            .checked_add(1)
            .ok_or("lease ids exhausted")?;
        Ok(LeaseId::try_from(self.next_lease)?)
    }
}

/// Builds a takeover from the wire request a client sends, epoch as a decimal string.
///
/// # Errors
///
/// Fails when the wire request does not parse.
pub fn takeover(slot: ControlSlot, epoch: LeaseGeneration, fresh: LeaseId) -> TestResult<Takeover> {
    let request: TakeoverRequest = serde_json::from_value(json!({
        "lease_epoch": epoch.get().to_string(),
        "reason": "explicit takeover",
    }))?;
    Ok(Takeover::new(slot, &request, fresh)?)
}

/// Lease events carry no snapshot section of their own.
#[derive(Debug)]
#[non_exhaustive]
pub struct NoSections;

impl SnapshotProducer for NoSections {
    type Sections = ();

    fn capture(&self, _snapshot: &Snapshot, _filter: &Filter) -> Result<(), EventError> {
        Ok(())
    }
}

/// A temporary store root removed after the coordinator using it closed.
#[derive(Debug)]
#[non_exhaustive]
pub struct Root(pub PathBuf);

impl Default for Root {
    fn default() -> Self {
        Self(std::env::temp_dir().join(format!("ovis-leases-{}", Uuid::now_v7())))
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.0);
        assert!(removed.is_ok(), "lease store cleanup: {removed:?}");
    }
}

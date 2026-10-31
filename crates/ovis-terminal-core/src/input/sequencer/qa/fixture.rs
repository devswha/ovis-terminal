//! Real authority, leases, VT modes and a live sequencer on one terminal.

use std::sync::Arc;

use ovis_terminal_authz::{
    Authority, AuthorityConfig, CredentialId, EndpointPolicy, FileEvidence, Grant, Issuance,
    IssueRequest, LocalAccount, OwnerSpec, PeerAdmission, PeerEvidence, Permission, Presentation,
    PrincipalId, ResolvedTarget, ResourceScope, Revocation, RuntimeBinding, ScopeResource, Session,
    TargetResource, UnixSocketEvidence, UnixUid,
};
use ovis_terminal_core::input::encoding::ModeSnapshot;
use ovis_terminal_core::input::receipts::Receipt;
use ovis_terminal_core::input::sequencer::{
    Binding, Expected, Moment, Payload, QueueLimits, Request, Sequencer, SequencerError,
    Submission, View, Writer,
};
use ovis_terminal_core::leases::{Acquire, ControlSlot, TerminalLeases};
use ovis_terminal_core::registry::{
    LeaseId, MachineId, ProcessGeneration, RegistryIdentities, RuntimeId, TerminalGeneration,
    TerminalId,
};
use ovis_terminal_holder::protocol::{Input, InputWritten};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::ClientId;
use ovis_terminal_protocol::schema::methods::server::WriteSequence;
use ovis_terminal_protocol::schema::scalar::{ByteOffset, TimestampMs};
use ovis_terminal_vt::{Geometry, HistoryLimit, Vt, VtConfig};
use uuid::Uuid;

use super::TestResult;

/// Request time of every fixture call.
pub const NOW: TimestampMs = TimestampMs::new(1_000);
/// Ordinary writer.
pub const WRITER: &[Permission] = &[Permission::Observe, Permission::Interact];
/// Writer with the raw-input grant.
pub const RAW: &[Permission] = &[
    Permission::Observe,
    Permission::Interact,
    Permission::RawInput,
];
/// Writer that may also take over.
pub const TAKEOVER: &[Permission] = &[
    Permission::Observe,
    Permission::Interact,
    Permission::Takeover,
];
/// Watcher only.
pub const OBSERVER: &[Permission] = &[Permission::Observe];

type Registry = RegistryIdentities;

/// One authenticated principal attached under the client label of the same name.
#[derive(Debug)]
pub struct Client {
    session: Session,
    label: ClientId,
    credential: CredentialId,
}

impl Client {
    /// Writes presenting no lease.
    pub const fn writer(&self) -> Writer<'_> {
        Writer::new(&self.session, &self.label, NOW)
    }

    /// The lease caller this client acts as.
    pub const fn caller(&self) -> ovis_terminal_core::leases::Caller<'_> {
        ovis_terminal_core::leases::Caller::new(&self.session, &self.label, NOW)
    }
}

/// A bootstrapped authority and one terminal target.
#[derive(Debug)]
pub struct Fixture {
    authority: Arc<Authority<Registry>>,
    target: ResolvedTarget<Registry>,
    admission: PeerAdmission,
    owner: Session,
    next_lease: u64,
}

/// The process the fixture terminal runs.
///
/// # Errors
///
/// Never for a nonzero value.
pub fn process(value: u64) -> TestResult<ProcessGeneration> {
    Ok(ProcessGeneration::new(value)?)
}

/// Modes of a fresh 80x24 emulator after `output`.
///
/// # Errors
///
/// Fails when the geometry is rejected.
pub fn modes(output: &[u8]) -> TestResult<ModeSnapshot> {
    let mut vt = Vt::new(VtConfig::new(Geometry::new(80, 24)?, HistoryLimit::new(10)));
    vt.feed(output);
    Ok(ModeSnapshot::capture(&vt))
}

/// The state of a check at the fixture instant.
pub const fn view<'a>(leases: &'a TerminalLeases, modes: &'a ModeSnapshot) -> View<'a> {
    View::new(leases, modes, Moment::new(NOW, ByteOffset::new(0)))
}

/// A submission under `op`.
pub fn submission<'a>(op: &str, expected: Expected, payload: Payload<'a>) -> Submission<'a> {
    Submission::new(OpId::new(op.to_owned()), expected, payload)
}

/// Admits `submission` for `writer`.
///
/// # Errors
///
/// Returns the sequencer's refusal.
pub fn admit(
    sequencer: &mut Sequencer,
    view: &View<'_>,
    request: (Writer<'_>, Submission<'_>),
) -> Result<Receipt, SequencerError> {
    sequencer.admit(view, Request::new(request.0, request.1))
}

/// What the holder reports after writing `written` bytes of `input`.
pub fn ack(input: &Input, written: u64) -> InputWritten {
    InputWritten {
        target: input.target.clone(),
        sequence: input.sequence,
        grant_epoch: input.grant.grant_epoch,
        fence_epoch: input.grant.fence_epoch,
        ack_epoch: input.sequence,
        written_bytes: written,
        output_offset: 0,
    }
}

impl Fixture {
    /// Bootstraps the authority.
    ///
    /// # Errors
    ///
    /// Fails when the authority refuses the setup step.
    pub fn new() -> TestResult<Self> {
        let machine = MachineId::new(Uuid::from_u128(25))?;
        let binding = RuntimeBinding::<Registry>::new(machine, RuntimeId::try_from(1)?);
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
            terminal: TerminalId::try_from(9)?,
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

    /// The fixture terminal as a resolved target.
    pub fn target(&self) -> ResolvedTarget<Registry> {
        self.target.clone()
    }

    /// What a request resolved against the fixture terminal expects.
    ///
    /// # Errors
    ///
    /// Never for the fixture runtime.
    pub fn expected(&self) -> TestResult<Expected> {
        Ok(Expected::new(RuntimeId::try_from(1)?, self.target()))
    }

    /// A fresh lease table.
    ///
    /// # Errors
    ///
    /// Fails when the target is rejected.
    pub fn leases(&self) -> TestResult<TerminalLeases> {
        Ok(TerminalLeases::new(
            Arc::clone(&self.authority),
            self.target(),
        )?)
    }

    /// The fixture terminal bound to the fixture authority.
    ///
    /// # Errors
    ///
    /// Fails when the target is rejected.
    pub fn binding(&self) -> TestResult<Binding> {
        Ok(Binding::new(
            Arc::clone(&self.authority),
            RuntimeId::try_from(1)?,
            self.target(),
        )?)
    }

    /// A sequencer whose stream is live with process 1.
    ///
    /// # Errors
    ///
    /// Fails when the target is rejected.
    pub fn sequencer(&self, limits: QueueLimits) -> TestResult<Sequencer> {
        let mut sequencer = Sequencer::new(self.binding()?, limits);
        let fenced = sequencer.stream_ready(process(1)?, WriteSequence::new(0));
        if !fenced.is_empty() {
            return Err("an empty sequencer fenced entries".into());
        }
        Ok(sequencer)
    }

    /// Issues `permissions` on the terminal to `label` and authenticates it.
    ///
    /// # Errors
    ///
    /// Fails when the authority refuses the setup step.
    pub fn client(&self, label: &str, permissions: &[Permission]) -> TestResult<Client> {
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
        let issued = self
            .authority
            .issue(&self.owner, Issuance::new(request, NOW))?;
        let session = self
            .authority
            .authenticate(&self.admission, Presentation::new(issued.bearer(), NOW))?;
        Ok(Client {
            session,
            label: ClientId::new(label.to_owned()),
            credential: *issued.id(),
        })
    }

    /// Revokes the credential `client` authenticated with.
    ///
    /// # Errors
    ///
    /// Fails when the authority refuses.
    pub fn revoke(&self, client: &Client) -> TestResult {
        self.authority
            .revoke(&self.owner, &Revocation::new(client.credential, NOW))?;
        Ok(())
    }

    /// Acquires the interact lease for `client` under a fresh identity.
    ///
    /// # Errors
    ///
    /// Fails when the lease table refuses.
    pub fn interact(
        &mut self,
        leases: &mut TerminalLeases,
        client: &Client,
    ) -> TestResult<LeaseId> {
        let fresh = self.fresh()?;
        leases.acquire(client.caller(), Acquire::new(ControlSlot::Interact, fresh))?;
        Ok(fresh)
    }

    /// A lease identity not handed out before.
    ///
    /// # Errors
    ///
    /// Fails when ids are exhausted.
    pub fn fresh(&mut self) -> TestResult<LeaseId> {
        self.next_lease = self
            .next_lease
            .checked_add(1)
            .ok_or("lease ids exhausted")?;
        Ok(LeaseId::try_from(self.next_lease)?)
    }
}

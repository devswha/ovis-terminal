//! Authority over principals, resource scopes and commit-time decision rechecks.
//!
//! `happy` and `failure` are the QA entry points named by the plan; the modules hold
//! the stale-state, forged-input and escalation classes.

#[path = "authority/issuance.rs"]
mod issuance;
#[path = "authority/reclamation.rs"]
mod reclamation;
#[path = "authority/revocation.rs"]
mod revocation;
#[path = "authority/scoping.rs"]
mod scoping;

use std::error::Error;

use ovis_terminal_authz::{
    AccessRequest, Action, Authority, AuthorityConfig, CommitCheck, CredentialEpoch, CredentialId,
    EndpointPolicy, FileEvidence, Grant, IdentityFamily, Issuance, IssueRequest, IssuedCredential,
    LocalAccount, OwnerSpec, PeerAdmission, PeerEvidence, Permission, Presentation, PrincipalId,
    ResolvedTarget, ResourceScope, Revocation, RuntimeBinding, ScopeResource, Session,
    TargetResource, UnixSocketEvidence, UnixUid,
};
use ovis_terminal_protocol::schema::ids::{
    DocName, MachineId, PaneId, PluginId, RuntimeLifetimeId, TabId, TerminalId, WorkspaceId,
};
use ovis_terminal_protocol::schema::scalar::{Incarnation, TimestampMs};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct WireIds;

impl IdentityFamily for WireIds {
    type Machine = MachineId;
    type Lifetime = RuntimeLifetimeId;
    type Workspace = WorkspaceId;
    type Tab = TabId;
    type Pane = PaneId;
    type Terminal = TerminalId;
    type Incarnation = Incarnation;
    type Document = DocName;
    type Plugin = PluginId;
}

fn target(resource: TargetResource<WireIds>) -> ResolvedTarget<WireIds> {
    ResolvedTarget::new(machine(), resource)
}

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const NOW: TimestampMs = TimestampMs::new(1_000_000);
const OWNER_EXPIRY: TimestampMs = TimestampMs::new(10_000_000);
const ISSUED_EXPIRY: TimestampMs = TimestampMs::new(5_000_000);

const OWNER_UID: u32 = 1000;

fn machine() -> MachineId {
    MachineId::new("m1".to_owned())
}

fn terminal_id(id: &str) -> TerminalId {
    TerminalId::new(id.to_owned())
}

fn grant(
    permission: ovis_terminal_authz::Permission,
    resource: ScopeResource<WireIds>,
) -> Grant<WireIds> {
    Grant::new(permission, ResourceScope::new(machine(), resource))
}

fn local_admission() -> TestResult<PeerAdmission> {
    let owner = UnixUid::new(OWNER_UID);
    let policy = EndpointPolicy::new(LocalAccount::Unix(owner));
    let evidence = UnixSocketEvidence::new(
        FileEvidence::new(owner, 0o600),
        FileEvidence::new(owner, 0o700),
        owner,
    );
    Ok(policy.admit(&PeerEvidence::UnixSocket(evidence))?)
}

struct Fixture {
    authority: Authority<WireIds>,
    owner: Session,
    owner_bearer: String,
}

impl Fixture {
    fn new() -> TestResult<Self> {
        Self::for_lifetime("lifetime-a")
    }

    fn for_lifetime(lifetime: &str) -> TestResult<Self> {
        let binding = RuntimeBinding::new(machine(), RuntimeLifetimeId::new(lifetime.to_owned()));
        let owner = OwnerSpec::new(PrincipalId::parse("owner")?, OWNER_EXPIRY);
        let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner))?;
        let owner_bearer = issued.bearer().to_owned();
        let session =
            authority.authenticate(&local_admission()?, Presentation::new(&owner_bearer, NOW))?;
        Ok(Self {
            authority,
            owner: session,
            owner_bearer,
        })
    }

    fn issue(
        &self,
        principal: &str,
        grants: Vec<Grant<WireIds>>,
    ) -> TestResult<IssuedCredential<WireIds>> {
        let request = IssueRequest::new(PrincipalId::parse(principal)?, grants, ISSUED_EXPIRY);
        Ok(self
            .authority
            .issue(&self.owner, Issuance::new(request, NOW))?)
    }

    fn revoke(&self, credential: CredentialId) -> TestResult<CredentialEpoch> {
        let revocation = Revocation::new(credential, NOW);
        Ok(self.authority.revoke(&self.owner, &revocation)?)
    }

    fn outcome(&self, session: &Session, request: &AccessRequest<WireIds>) -> &'static str {
        match self.authority.authorize(session, request) {
            Ok(_) => "allowed",
            Err(error) => error.kind(),
        }
    }

    fn session(
        &self,
        principal: &str,
        grants: Vec<Grant<WireIds>>,
    ) -> TestResult<(Session, IssuedCredential<WireIds>)> {
        let issued = self.issue(principal, grants)?;
        let session = self
            .authority
            .authenticate(&local_admission()?, Presentation::new(issued.bearer(), NOW))?;
        Ok((session, issued))
    }
}

fn terminal(id: &str) -> ResolvedTarget<WireIds> {
    target(TargetResource::Terminal {
        placement: None,
        terminal: terminal_id(id),
        incarnation: Incarnation::new(1),
    })
}

fn workspace(id: &str) -> ResolvedTarget<WireIds> {
    target(TargetResource::Workspace {
        workspace: WorkspaceId::new(id.to_owned()),
    })
}

fn document(name: &str) -> ResolvedTarget<WireIds> {
    target(TargetResource::Document {
        name: DocName::new(name.to_owned()),
    })
}

fn terminal_scope(id: &str) -> ScopeResource<WireIds> {
    ScopeResource::Terminal {
        id: terminal_id(id),
    }
}

#[test]
fn happy() -> TestResult {
    let fixture = Fixture::new()?;
    let (observer, _) = fixture.session(
        "phone",
        vec![grant(Permission::Observe, ScopeResource::Machine)],
    )?;
    let (operator, _) = fixture.session(
        "agent-t1",
        vec![
            grant(Permission::Observe, terminal_scope("t1")),
            grant(Permission::Interact, terminal_scope("t1")),
        ],
    )?;

    for target in [terminal("t1"), terminal("t2")] {
        for action in [Action::Read, Action::Subscribe] {
            let request = AccessRequest::new(action, target.clone(), NOW);
            let decision = fixture.authority.authorize(&observer, &request)?;
            assert_eq!(decision.principal().as_str(), "phone");
            assert_eq!(decision.target(), &target);
            fixture
                .authority
                .recheck(&decision, &CommitCheck::new(action, target.clone(), NOW))?;
        }
    }
    let observer_write = fixture
        .authority
        .authorize(
            &observer,
            &AccessRequest::new(Action::Input, terminal("t1"), NOW),
        )
        .map(|_| ());
    let observer_write = observer_write
        .err()
        .ok_or("observer input was authorized")?;
    assert_eq!(observer_write.kind(), "forbidden");

    for action in [Action::Input, Action::Resize] {
        let decision = fixture
            .authority
            .authorize(&operator, &AccessRequest::new(action, terminal("t1"), NOW))?;
        fixture
            .authority
            .recheck(&decision, &CommitCheck::new(action, terminal("t1"), NOW))?;
    }
    let outside = [
        AccessRequest::new(Action::Input, terminal("t2"), NOW),
        AccessRequest::new(Action::Launch, workspace("w1"), NOW),
    ];
    for request in &outside {
        let refused = fixture.authority.authorize(&operator, request).map(|_| ());
        let refused = refused.err().ok_or("operator acted outside its terminal")?;
        assert_eq!(refused.kind(), "forbidden");
    }
    println!(
        "observer read+subscribe t1,t2 ok; observer input -> {}; operator input+resize t1 ok; \
         operator outside t1 -> forbidden",
        observer_write.kind()
    );
    Ok(())
}

#[test]
fn failure() -> TestResult {
    let fixture = Fixture::new()?;
    let (operator, operator_credential) = fixture.session(
        "agent-t1",
        vec![grant(Permission::Interact, terminal_scope("t1"))],
    )?;
    let queued = fixture.authority.authorize(
        &operator,
        &AccessRequest::new(Action::Input, terminal("t1"), NOW),
    )?;
    fixture.revoke(*operator_credential.id())?;
    let dispatched = fixture.authority.recheck(
        &queued,
        &CommitCheck::new(Action::Input, terminal("t1"), NOW),
    );
    let dispatched = dispatched.err().ok_or("revoked credential still wrote")?;
    assert_eq!(dispatched.kind(), "credential_revoked");
    println!("queued input after revocation -> {}", dispatched.kind());

    let (observer, _) = fixture.session(
        "phone",
        vec![grant(Permission::Observe, ScopeResource::Machine)],
    )?;
    let attempts = [
        (Action::Input, terminal("t1")),
        (Action::Resize, terminal("t1")),
        (Action::Launch, workspace("w1")),
        (Action::DocumentWrite, document("notes")),
        (Action::LayoutChange, workspace("w1")),
        (Action::Takeover, terminal("t1")),
        (Action::PromptAnswer, terminal("t1")),
    ];
    for (action, target) in attempts {
        let refused = fixture
            .authority
            .authorize(&observer, &AccessRequest::new(action, target, NOW))
            .map(|_| ());
        let refused = refused.err().ok_or("observer mutation was authorized")?;
        assert_eq!(refused.kind(), "forbidden", "{action}");
        println!("observer {action} -> {}", refused.kind());
    }
    Ok(())
}

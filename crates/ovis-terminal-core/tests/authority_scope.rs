//! Registry-issued identities flow into authority scopes and resolved targets and come back
//! out of decisions as the same typed values, with no string conversion at the seam.

use std::error::Error;
use std::sync::atomic::{AtomicU64, Ordering};

use ovis_terminal_authz::{
    AccessRequest, Action, Authority, AuthorityConfig, EndpointPolicy, FileEvidence, Grant,
    Issuance, IssueRequest, LocalAccount, OwnerSpec, PeerEvidence, Permission, Placement,
    Presentation, PrincipalId, ResolvedTarget, ResourceScope, RuntimeBinding, ScopeResource,
    Session, TargetResource, UnixSocketEvidence, UnixUid,
};
use ovis_terminal_core::registry::{
    IdentityReservation, MachineId, PaneId, RegistryIdentities, RuntimeId, TabId,
    TerminalGeneration, TerminalId, WorkspaceId,
};
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_store::{OpId, Store, StoreOptions};
use serde_json::json;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
type Registry = RegistryIdentities;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);
const NOW: TimestampMs = TimestampMs::new(1_000);

struct Reserved {
    machine: MachineId,
    workspace: WorkspaceId,
    tab: TabId,
    pane: PaneId,
    granted: TerminalId,
    other: TerminalId,
}

async fn reserve() -> TestResult<Reserved> {
    let root = std::env::temp_dir().join(format!(
        "ovis-authority-scope-{}-{}",
        std::process::id(),
        NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
    ));
    let store = Store::open(&root, StoreOptions::default())?;
    let ids = IdentityReservation::new(&store);
    let reserved = Reserved {
        machine: ids.load_or_reserve_machine(OpId::new("machine")).await?,
        workspace: ids.reserve::<WorkspaceId>(OpId::new("workspace")).await?,
        tab: ids.reserve::<TabId>(OpId::new("tab")).await?,
        pane: ids.reserve::<PaneId>(OpId::new("pane")).await?,
        granted: ids.reserve::<TerminalId>(OpId::new("granted")).await?,
        other: ids.reserve::<TerminalId>(OpId::new("other")).await?,
    };
    store.close()?;
    std::fs::remove_dir_all(&root)?;
    Ok(reserved)
}

fn agent_session(ids: &Reserved) -> TestResult<(Authority<Registry>, Session)> {
    let binding = RuntimeBinding::<Registry>::new(ids.machine, RuntimeId::try_from(1_u64)?);
    let owner = OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(10_000));
    let (authority, owner_credential) = Authority::bootstrap(AuthorityConfig::new(binding, owner))?;
    let uid = UnixUid::new(1000);
    let socket = UnixSocketEvidence::new(
        FileEvidence::new(uid, 0o600),
        FileEvidence::new(uid, 0o700),
        uid,
    );
    let admission =
        EndpointPolicy::new(LocalAccount::Unix(uid)).admit(&PeerEvidence::UnixSocket(socket))?;
    let owner_session = authority.authenticate(
        &admission,
        Presentation::new(owner_credential.bearer(), NOW),
    )?;
    let scope = |resource| ResourceScope::new(ids.machine, resource);
    let grants = vec![
        Grant::new(
            Permission::Interact,
            scope(ScopeResource::Terminal { id: ids.granted }),
        ),
        Grant::new(
            Permission::Observe,
            scope(ScopeResource::Workspace { id: ids.workspace }),
        ),
    ];
    let request = IssueRequest::new(
        PrincipalId::parse("agent")?,
        grants,
        TimestampMs::new(5_000),
    );
    let issued = authority.issue(&owner_session, Issuance::new(request, NOW))?;
    let agent = authority.authenticate(&admission, Presentation::new(issued.bearer(), NOW))?;
    Ok((authority, agent))
}

#[tokio::test]
async fn registry_identities_resolve_into_scopes_and_back() -> TestResult {
    let ids = reserve().await?;
    let (authority, agent) = agent_session(&ids)?;
    let incarnation = TerminalGeneration::new(1)?;
    let on = |action, terminal, placement| {
        let resource = TargetResource::Terminal {
            placement,
            terminal,
            incarnation,
        };
        AccessRequest::new(action, ResolvedTarget::new(ids.machine, resource), NOW)
    };
    let placed = || Some(Placement::new(ids.workspace, ids.tab, ids.pane));

    let decision = authority.authorize(&agent, &on(Action::Input, ids.granted, None))?;
    let other_input = authority.authorize(&agent, &on(Action::Input, ids.other, None));
    let placed_read = authority.authorize(&agent, &on(Action::Read, ids.other, placed()));
    let unplaced_read = authority.authorize(&agent, &on(Action::Read, ids.other, None));

    let TargetResource::Terminal { terminal, .. } = decision.target().resource() else {
        return Err("decision target is not a terminal".into());
    };
    assert_eq!(*terminal, ids.granted);
    assert_eq!(*decision.target().machine(), ids.machine);
    assert_eq!(
        other_input.err().map(|error| error.kind()),
        Some("forbidden")
    );
    assert!(
        placed_read.is_ok(),
        "workspace grant must reach a terminal placed in it"
    );
    assert_eq!(
        unplaced_read.err().map(|error| error.kind()),
        Some("forbidden")
    );
    Ok(())
}

#[test]
fn issuance_document_rejects_an_identity_of_another_family() {
    let document = |id: &str| {
        json!({
            "principal": "agent",
            "expires_at_ms": "5000",
            "grants": [{ "permission": "observe", "scope": {
                "machine": "machine:67e55044-10b1-426f-9247-bb680e5fe0c8",
                "resource": { "kind": "terminal", "id": id } } }]
        })
        .to_string()
    };

    let terminal = IssueRequest::<Registry>::from_json(&document("terminal:7"));
    let tab = IssueRequest::<Registry>::from_json(&document("tab:7"));

    assert!(terminal.is_ok());
    let rejected = tab.err();
    assert_eq!(
        rejected.as_ref().map(ovis_terminal_authz::AuthzError::kind),
        Some("invalid_params")
    );
    assert_eq!(
        rejected.as_ref().and_then(|error| error.field_path()),
        Some("grants[0].scope.resource")
    );
}

use ovis_terminal_authz::{
    AccessRequest, Action, EndpointPolicy, LocalAccount, PeerEvidence, Permission, Placement,
    Presentation, RemoteTransport, ResolvedTarget, ScopeResource, TargetResource, Transport,
    UnixUid,
};
use ovis_terminal_protocol::schema::ids::{PaneId, PluginId, TabId, WorkspaceId};
use ovis_terminal_protocol::schema::scalar::Incarnation;

use super::{
    Fixture, NOW, TestResult, grant, local_admission, machine, terminal, terminal_id,
    terminal_scope,
};

fn placed_terminal(id: &str, workspace: &str) -> ResolvedTarget {
    let placement = Placement::new(
        WorkspaceId::new(workspace.to_owned()),
        TabId::new(format!("{workspace}-tab")),
        PaneId::new(format!("{workspace}-pane")),
    );
    ResolvedTarget::new(
        machine(),
        TargetResource::Terminal {
            placement: Some(placement),
            terminal: terminal_id(id),
            incarnation: Incarnation::new(1),
        },
    )
}

fn plugin(id: &str) -> ResolvedTarget {
    ResolvedTarget::new(
        machine(),
        TargetResource::Plugin {
            plugin: PluginId::new(id.to_owned()),
        },
    )
}

fn machine_target() -> ResolvedTarget {
    ResolvedTarget::new(machine(), TargetResource::Machine)
}

const fn ask(action: Action, target: ResolvedTarget) -> AccessRequest {
    AccessRequest::new(action, target, NOW)
}

#[test]
fn workspace_scope_covers_only_terminals_placed_in_that_workspace() -> TestResult {
    let fixture = Fixture::new()?;
    let workspace = ScopeResource::Workspace {
        id: WorkspaceId::new("w1".to_owned()),
    };
    let (session, _) = fixture.session("agent-w1", vec![grant(Permission::Interact, workspace)])?;

    let inside = fixture.outcome(&session, &ask(Action::Input, placed_terminal("t1", "w1")));
    let other_workspace =
        fixture.outcome(&session, &ask(Action::Input, placed_terminal("t2", "w2")));
    let unplaced = fixture.outcome(&session, &ask(Action::Input, terminal("t3")));

    assert_eq!(
        [inside, other_workspace, unplaced],
        ["allowed", "forbidden", "forbidden"]
    );
    Ok(())
}

#[test]
fn takeover_and_prompt_answer_need_independent_grants() -> TestResult {
    let fixture = Fixture::new()?;
    let (interactor, _) = fixture.session(
        "bridge",
        vec![
            grant(Permission::Observe, ScopeResource::Machine),
            grant(Permission::Interact, ScopeResource::Machine),
        ],
    )?;
    let (taker, _) = fixture.session(
        "operator",
        vec![grant(Permission::Takeover, terminal_scope("t1"))],
    )?;

    let interactor_takeover = fixture.outcome(&interactor, &ask(Action::Takeover, terminal("t1")));
    let interactor_answer =
        fixture.outcome(&interactor, &ask(Action::PromptAnswer, terminal("t1")));
    let taker_takeover = fixture.outcome(&taker, &ask(Action::Takeover, terminal("t1")));
    let taker_answer = fixture.outcome(&taker, &ask(Action::PromptAnswer, terminal("t1")));

    assert_eq!(
        [
            interactor_takeover,
            interactor_answer,
            taker_takeover,
            taker_answer,
        ],
        ["forbidden", "forbidden", "allowed", "forbidden"]
    );
    Ok(())
}

#[test]
fn raw_frames_need_an_explicit_raw_input_grant() -> TestResult {
    let fixture = Fixture::new()?;
    let (interactor, _) = fixture.session(
        "agent-t1",
        vec![grant(Permission::Interact, terminal_scope("t1"))],
    )?;
    let (raw, _) = fixture.session(
        "raw-t1",
        vec![grant(Permission::RawInput, terminal_scope("t1"))],
    )?;

    let interactor_raw = fixture.outcome(&interactor, &ask(Action::RawInput, terminal("t1")));
    let granted_raw = fixture.outcome(&raw, &ask(Action::RawInput, terminal("t1")));
    let granted_raw_elsewhere = fixture.outcome(&raw, &ask(Action::RawInput, terminal("t2")));

    assert_eq!(
        [interactor_raw, granted_raw, granted_raw_elsewhere],
        ["forbidden", "allowed", "forbidden"]
    );
    Ok(())
}

#[test]
fn plugin_and_admin_scopes_are_separate() -> TestResult {
    let fixture = Fixture::new()?;
    let (admin, _) = fixture.session(
        "admin",
        vec![grant(Permission::Admin, ScopeResource::Machine)],
    )?;
    let plugin_scope = ScopeResource::Plugin {
        id: PluginId::new("p1".to_owned()),
    };
    let (runner, _) =
        fixture.session("plugin-p1", vec![grant(Permission::Plugin, plugin_scope)])?;

    let results = [
        fixture.outcome(&admin, &ask(Action::PluginInvoke, plugin("p1"))),
        fixture.outcome(&admin, &ask(Action::Administer, machine_target())),
        fixture.outcome(&runner, &ask(Action::PluginInvoke, plugin("p1"))),
        fixture.outcome(&runner, &ask(Action::PluginInvoke, plugin("p2"))),
        fixture.outcome(&runner, &ask(Action::Administer, machine_target())),
        fixture.outcome(&runner, &ask(Action::Input, terminal("t1"))),
    ];

    assert_eq!(
        results,
        [
            "forbidden",
            "allowed",
            "allowed",
            "forbidden",
            "forbidden",
            "forbidden"
        ]
    );
    Ok(())
}

#[test]
fn endpoint_possession_without_credential_material_grants_nothing() -> TestResult {
    let fixture = Fixture::new()?;
    let (_, issued) = fixture.session(
        "phone",
        vec![grant(Permission::Observe, ScopeResource::Machine)],
    )?;
    let policy = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1000)));
    let remote = policy.admit(&PeerEvidence::Remote(RemoteTransport::WebSocket))?;

    let local_bare = fixture
        .authority
        .authenticate(&local_admission()?, Presentation::new("", NOW));
    let remote_bare = fixture
        .authority
        .authenticate(&remote, Presentation::new("", NOW));
    let remote_session = fixture
        .authority
        .authenticate(&remote, Presentation::new(issued.bearer(), NOW))?;

    assert_eq!(
        local_bare.err().map(|error| error.kind()),
        Some("unauthenticated")
    );
    assert_eq!(
        remote_bare.err().map(|error| error.kind()),
        Some("unauthenticated")
    );
    assert_eq!(remote_session.transport(), Transport::WebSocket);
    assert_eq!(
        fixture.outcome(&remote_session, &ask(Action::Read, terminal("t1"))),
        "allowed"
    );
    assert_eq!(
        fixture.outcome(&remote_session, &ask(Action::Input, terminal("t1"))),
        "forbidden"
    );
    Ok(())
}

#[test]
fn credential_without_grants_is_denied_every_action() -> TestResult {
    let fixture = Fixture::new()?;
    let (session, _) = fixture.session("empty", Vec::new())?;

    let read = fixture.outcome(&session, &ask(Action::Read, terminal("t1")));
    let subscribe = fixture.outcome(&session, &ask(Action::Subscribe, machine_target()));

    assert_eq!([read, subscribe], ["forbidden", "forbidden"]);
    Ok(())
}

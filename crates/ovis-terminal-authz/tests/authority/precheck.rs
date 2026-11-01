use ovis_terminal_authz::{AccessRequest, Action, Permission, ScopeResource, TargetResource};

use super::{Fixture, NOW, TestResult, grant, target, terminal_id};

#[test]
fn an_observer_is_conferred_observe_and_nothing_that_mutates() -> TestResult {
    let fixture = Fixture::new()?;
    let (observer, _) = fixture.session(
        "observer",
        vec![grant(Permission::Observe, ScopeResource::Machine)],
    )?;

    let conferred = [
        Permission::Observe,
        Permission::Interact,
        Permission::RawInput,
        Permission::Takeover,
        Permission::PromptAnswer,
        Permission::Report,
        Permission::Admin,
        Permission::Plugin,
    ]
    .map(|permission| fixture.authority.confers(&observer, (permission, NOW)).ok());

    assert_eq!(
        conferred,
        [
            Some(true),
            Some(false),
            Some(false),
            Some(false),
            Some(false),
            Some(false),
            Some(false),
            Some(false)
        ]
    );
    Ok(())
}

#[test]
fn a_terminal_scoped_grant_is_conferred_although_the_machine_is_forbidden() -> TestResult {
    let fixture = Fixture::new()?;
    let scope = ScopeResource::Terminal {
        id: terminal_id("t1"),
    };
    let (agent, _) = fixture.session("agent-t1", vec![grant(Permission::Interact, scope)])?;
    let machine_wide = AccessRequest::new(Action::Input, target(TargetResource::Machine), NOW);

    let conferred = fixture
        .authority
        .confers(&agent, (Permission::Interact, NOW))?;
    let decided = fixture.authority.authorize(&agent, &machine_wide);

    assert!(conferred);
    assert_eq!(decided.err().map(|error| error.kind()), Some("forbidden"));
    Ok(())
}

#[test]
fn a_revoked_credential_confers_nothing_and_reports_its_revocation() -> TestResult {
    let fixture = Fixture::new()?;
    let (writer, issued) = fixture.session(
        "writer",
        vec![grant(Permission::Interact, ScopeResource::Machine)],
    )?;
    fixture.revoke(*issued.id())?;

    let conferred = fixture
        .authority
        .confers(&writer, (Permission::Interact, NOW));

    assert_eq!(
        conferred.err().map(|error| error.kind()),
        Some("credential_revoked")
    );
    Ok(())
}

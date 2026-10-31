use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ovis_terminal_authz::{
    AccessRequest, Action, Authority, AuthorityConfig, AuthorityLimits, AuthzError, CommitCheck,
    Issuance, IssueRequest, OwnerSpec, Permission, Presentation, PrincipalId, ResolvedTarget,
    Revocation, RuntimeBinding, ScopeResource, TargetResource,
};
use ovis_terminal_protocol::schema::ids::RuntimeLifetimeId;
use ovis_terminal_protocol::schema::scalar::Incarnation;

use super::{
    Fixture, ISSUED_EXPIRY, NOW, OWNER_EXPIRY, TestResult, grant, local_admission, machine,
    terminal, terminal_id, terminal_scope,
};

fn kind_of<T>(result: Result<T, AuthzError>) -> TestResult<&'static str> {
    Ok(result.err().ok_or("expected a typed rejection")?.kind())
}

fn interact_t1(
    fixture: &Fixture,
) -> TestResult<(
    ovis_terminal_authz::Session,
    ovis_terminal_authz::IssuedCredential,
)> {
    fixture.session(
        "agent-t1",
        vec![grant(Permission::Interact, terminal_scope("t1"))],
    )
}

#[test]
fn expiry_between_decision_and_commit_rejects_the_write() -> TestResult {
    let fixture = Fixture::new()?;
    let (session, issued) = interact_t1(&fixture)?;
    let decision = fixture.authority.authorize(
        &session,
        &AccessRequest::new(Action::Input, terminal("t1"), NOW),
    )?;

    let at_expiry = CommitCheck::new(Action::Input, terminal("t1"), ISSUED_EXPIRY);
    let commit = fixture.authority.recheck(&decision, &at_expiry);
    let late_login = fixture.authority.authenticate(
        &local_admission()?,
        Presentation::new(issued.bearer(), ISSUED_EXPIRY),
    );

    assert_eq!(kind_of(commit)?, "credential_expired");
    assert_eq!(kind_of(late_login)?, "credential_expired");
    Ok(())
}

#[test]
fn revoked_credential_cannot_authenticate_or_authorize_again() -> TestResult {
    let fixture = Fixture::new()?;
    let (session, issued) = interact_t1(&fixture)?;

    let epoch = fixture.revoke(*issued.id())?;

    assert!(epoch > issued.epoch(), "revocation must advance the epoch");
    let relogin = fixture
        .authority
        .authenticate(&local_admission()?, Presentation::new(issued.bearer(), NOW));
    let reuse = fixture.authority.authorize(
        &session,
        &AccessRequest::new(Action::Input, terminal("t1"), NOW),
    );
    assert_eq!(kind_of(relogin)?, "credential_revoked");
    assert_eq!(kind_of(reuse)?, "credential_revoked");
    Ok(())
}

#[test]
fn commit_must_match_the_decided_action_target_and_incarnation() -> TestResult {
    let fixture = Fixture::new()?;
    let (session, _) = fixture.session(
        "agent-t1",
        vec![
            grant(Permission::Observe, terminal_scope("t1")),
            grant(Permission::Interact, terminal_scope("t1")),
        ],
    )?;
    let read = fixture.authority.authorize(
        &session,
        &AccessRequest::new(Action::Read, terminal("t1"), NOW),
    )?;
    let input = fixture.authority.authorize(
        &session,
        &AccessRequest::new(Action::Input, terminal("t1"), NOW),
    )?;
    let replaced = ResolvedTarget::new(
        machine(),
        TargetResource::Terminal {
            placement: None,
            terminal: terminal_id("t1"),
            incarnation: Incarnation::new(2),
        },
    );

    let read_as_write = fixture
        .authority
        .recheck(&read, &CommitCheck::new(Action::Input, terminal("t1"), NOW));
    let other_terminal = fixture.authority.recheck(
        &input,
        &CommitCheck::new(Action::Input, terminal("t2"), NOW),
    );
    let new_incarnation = fixture
        .authority
        .recheck(&input, &CommitCheck::new(Action::Input, replaced, NOW));

    assert_eq!(kind_of(read_as_write)?, "decision_mismatch");
    assert_eq!(kind_of(other_terminal)?, "decision_mismatch");
    assert_eq!(kind_of(new_incarnation)?, "decision_mismatch");
    Ok(())
}

#[test]
fn decision_from_another_authority_is_rejected() -> TestResult {
    let previous_boot = Fixture::new()?;
    let current = Fixture::new()?;
    let (session, _) = interact_t1(&previous_boot)?;
    let decision = previous_boot.authority.authorize(
        &session,
        &AccessRequest::new(Action::Input, terminal("t1"), NOW),
    )?;

    let commit = current.authority.recheck(
        &decision,
        &CommitCheck::new(Action::Input, terminal("t1"), NOW),
    );

    assert_eq!(kind_of(commit)?, "decision_mismatch");
    Ok(())
}

#[test]
fn credential_is_scoped_to_its_runtime_lifetime() -> TestResult {
    let first = Fixture::for_lifetime("lifetime-a")?;
    let second = Fixture::for_lifetime("lifetime-b")?;
    let (_, issued_first) = interact_t1(&first)?;
    let (_, issued_second) = interact_t1(&second)?;
    assert_eq!(
        issued_first.id(),
        issued_second.id(),
        "fixture needs colliding ids"
    );

    let foreign_login = second.authority.authenticate(
        &local_admission()?,
        Presentation::new(issued_first.bearer(), NOW),
    );
    let foreign_owner = second.authority.authenticate(
        &local_admission()?,
        Presentation::new(&first.owner_bearer, NOW),
    );

    assert_eq!(kind_of(foreign_login)?, "invalid_credential");
    assert_eq!(kind_of(foreign_owner)?, "invalid_credential");
    Ok(())
}

#[test]
fn revocation_reaches_a_recheck_on_another_thread() -> TestResult {
    let fixture = Fixture::new()?;
    let (session, issued) = interact_t1(&fixture)?;
    let decision = fixture.authority.authorize(
        &session,
        &AccessRequest::new(Action::Input, terminal("t1"), NOW),
    )?;
    let (revoked_tx, revoked_rx) = mpsc::channel::<()>();

    let (authority, decision) = (&fixture.authority, &decision);
    let outcome = thread::scope(|scope| -> TestResult<Result<(), &'static str>> {
        let writer = scope.spawn(move || {
            revoked_rx.recv_timeout(Duration::from_secs(10)).map(|()| {
                authority
                    .recheck(
                        decision,
                        &CommitCheck::new(Action::Input, terminal("t1"), NOW),
                    )
                    .map_err(|error| error.kind())
            })
        });
        fixture.revoke(*issued.id())?;
        revoked_tx.send(())?;
        Ok(writer.join().map_err(|_| "writer thread panicked")??)
    })?;

    assert_eq!(outcome, Err("credential_revoked"));
    Ok(())
}

#[test]
fn credential_table_is_bounded_and_reclaims_revoked_entries() -> TestResult {
    let binding = RuntimeBinding::new(machine(), RuntimeLifetimeId::new("bounded".to_owned()));
    let owner = OwnerSpec::new(PrincipalId::parse("owner")?, OWNER_EXPIRY);
    let limits = AuthorityLimits::DEFAULT
        .with_max_credentials(2)
        .with_max_grants(8);
    let (authority, issued) =
        Authority::bootstrap(AuthorityConfig::new(binding, owner).with_limits(limits))?;
    let owner =
        authority.authenticate(&local_admission()?, Presentation::new(issued.bearer(), NOW))?;
    let request = |grants: usize| -> TestResult<Issuance> {
        let grants = vec![grant(Permission::Observe, ScopeResource::Machine); grants];
        Ok(Issuance::new(
            IssueRequest::new(PrincipalId::parse("agent")?, grants, ISSUED_EXPIRY),
            NOW,
        ))
    };

    let first = authority.issue(&owner, request(1)?)?;
    let full = authority.issue(&owner, request(1)?);
    authority.revoke(&owner, &Revocation::new(*first.id(), NOW))?;
    let reclaimed = authority.issue(&owner, request(1)?);
    let too_many_grants = authority.issue(&owner, request(9)?);

    assert_eq!(kind_of(full)?, "limit_exceeded");
    assert!(reclaimed.is_ok(), "revoked entry was not reclaimed");
    assert_eq!(kind_of(too_many_grants)?, "limit_exceeded");
    Ok(())
}

#[test]
fn machine_wide_grant_does_not_cover_another_machine() -> TestResult {
    let fixture = Fixture::new()?;
    let (session, _) = fixture.session(
        "phone",
        vec![grant(Permission::Observe, ScopeResource::Machine)],
    )?;
    let remote = ResolvedTarget::new(
        ovis_terminal_protocol::schema::ids::MachineId::new("m2".to_owned()),
        TargetResource::Machine,
    );

    let read = fixture
        .authority
        .authorize(&session, &AccessRequest::new(Action::Read, remote, NOW));

    assert_eq!(kind_of(read)?, "forbidden");
    Ok(())
}

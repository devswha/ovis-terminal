use ovis_terminal_authz::{
    AccessRequest, Action, Authority, AuthorityConfig, AuthorityLimits, AuthzError, CommitCheck,
    Issuance, IssueRequest, OwnerSpec, Permission, Presentation, PrincipalId, Revocation,
    RuntimeBinding, ScopeResource, Session,
};
use ovis_terminal_protocol::schema::ids::RuntimeLifetimeId;
use ovis_terminal_protocol::schema::scalar::TimestampMs;

use super::{
    ISSUED_EXPIRY, NOW, OWNER_EXPIRY, TestResult, WireIds, grant, local_admission, machine,
    terminal, terminal_scope,
};

fn kind_of<T>(result: Result<T, AuthzError>) -> &'static str {
    result.err().map_or("accepted", |error| error.kind())
}

fn bounded(limits: AuthorityLimits) -> TestResult<(Authority<WireIds>, Session)> {
    let binding = RuntimeBinding::new(machine(), RuntimeLifetimeId::new("bounded".to_owned()));
    let owner = OwnerSpec::new(PrincipalId::parse("owner")?, OWNER_EXPIRY);
    let (authority, issued) =
        Authority::bootstrap(AuthorityConfig::new(binding, owner).with_limits(limits))?;
    let owner =
        authority.authenticate(&local_admission()?, Presentation::new(issued.bearer(), NOW))?;
    Ok((authority, owner))
}

fn interact(terminal_id: &str, expires: TimestampMs) -> TestResult<IssueRequest<WireIds>> {
    let grants = vec![grant(Permission::Interact, terminal_scope(terminal_id))];
    Ok(IssueRequest::new(
        PrincipalId::parse("agent")?,
        grants,
        expires,
    ))
}

fn input_on(terminal_id: &str) -> AccessRequest<WireIds> {
    CommitCheck::new(Action::Input, terminal(terminal_id), NOW)
}

#[test]
fn credential_table_is_bounded_and_reclaims_revoked_entries() -> TestResult {
    let limits = AuthorityLimits::DEFAULT
        .with_max_credentials(2)
        .with_max_grants(8);
    let (authority, owner) = bounded(limits)?;
    let observers = |count: usize| -> TestResult<Issuance<WireIds>> {
        let grants = vec![grant(Permission::Observe, ScopeResource::Machine); count];
        let request = IssueRequest::new(PrincipalId::parse("agent")?, grants, ISSUED_EXPIRY);
        Ok(Issuance::new(request, NOW))
    };

    let first = authority.issue(&owner, observers(1)?)?;
    let full = authority.issue(&owner, observers(1)?);
    authority.revoke(&owner, &Revocation::new(*first.id(), NOW))?;
    let reclaimed = authority.issue(&owner, observers(1)?);
    let too_many_grants = authority.issue(&owner, observers(9)?);

    assert_eq!(kind_of(full), "limit_exceeded");
    assert!(reclaimed.is_ok(), "revoked entry was not reclaimed");
    assert_eq!(kind_of(too_many_grants), "limit_exceeded");
    Ok(())
}

#[test]
fn queued_decision_stays_revoked_after_its_record_is_reclaimed() -> TestResult {
    let (authority, owner) = bounded(AuthorityLimits::DEFAULT.with_max_credentials(2))?;
    let issued = authority.issue(&owner, Issuance::new(interact("t1", ISSUED_EXPIRY)?, NOW))?;
    let agent =
        authority.authenticate(&local_admission()?, Presentation::new(issued.bearer(), NOW))?;
    let queued = authority.authorize(&agent, &input_on("t1"))?;
    let revoked_epoch = authority.revoke(&owner, &Revocation::new(*issued.id(), NOW))?;

    let refill = authority.issue(&owner, Issuance::new(interact("t2", ISSUED_EXPIRY)?, NOW));
    let dispatched = authority.recheck(&queued, &input_on("t1"));
    let reused_session = authority.authorize(&agent, &input_on("t1"));
    let revoked_again = authority.revoke(&owner, &Revocation::new(*issued.id(), NOW))?;

    assert!(
        refill.is_ok(),
        "the revoked record was not reclaimed for the refill"
    );
    assert_eq!(kind_of(dispatched), "credential_revoked");
    assert_eq!(kind_of(reused_session), "credential_revoked");
    assert_eq!(revoked_again, revoked_epoch);
    Ok(())
}

#[test]
fn tombstone_bound_holds_unexpired_revocations_until_one_expires() -> TestResult {
    let limits = AuthorityLimits::DEFAULT
        .with_max_credentials(2)
        .with_max_tombstones(1);
    let (authority, owner) = bounded(limits)?;
    let (early, late, later) = (
        TimestampMs::new(2_000_000),
        TimestampMs::new(5_000_000),
        TimestampMs::new(4_000_000),
    );
    let first = authority.issue(&owner, Issuance::new(interact("t1", early)?, NOW))?;
    authority.revoke(&owner, &Revocation::new(*first.id(), NOW))?;
    let second = authority.issue(&owner, Issuance::new(interact("t2", late)?, NOW))?;
    let agent =
        authority.authenticate(&local_admission()?, Presentation::new(second.bearer(), NOW))?;
    let queued = authority.authorize(&agent, &input_on("t2"))?;
    authority.revoke(&owner, &Revocation::new(*second.id(), NOW))?;

    let tombstones_full = authority.issue(&owner, Issuance::new(interact("t3", later)?, NOW));
    let after_first_expiry = authority.issue(&owner, Issuance::new(interact("t3", later)?, early));
    let dispatched = authority.recheck(&queued, &input_on("t2"));

    assert_eq!(kind_of(tombstones_full), "limit_exceeded");
    assert!(
        after_first_expiry.is_ok(),
        "expired tombstone was not purged"
    );
    assert_eq!(kind_of(dispatched), "credential_revoked");
    Ok(())
}

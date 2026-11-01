//! Rotation, withdrawal and per-request revalidation: no socket keeps authority.

use ovis_terminal_authz::{
    AccessRequest, Action, CommitCheck, Permission, Presentation, PrincipalEpoch, Revocation,
    Rotation, ScopeResource,
};
use ovis_terminal_protocol::schema::scalar::TimestampMs;

use super::{
    Fixture, ISSUED_EXPIRY, NOW, OWNER_EXPIRY, TestResult, grant, local_admission, terminal,
};

fn writer_grants() -> Vec<ovis_terminal_authz::Grant<super::WireIds>> {
    vec![
        grant(Permission::Observe, ScopeResource::Machine),
        grant(Permission::Interact, ScopeResource::Machine),
        grant(Permission::PromptAnswer, ScopeResource::Machine),
    ]
}

#[test]
fn rotation_swaps_credentials_atomically_and_advances_the_principal_epoch() -> TestResult {
    // Given a writer session with a write decided but not yet committed.
    let fixture = Fixture::new()?;
    let (session, issued) = fixture.session("writer", writer_grants())?;
    let queued = fixture.authority.authorize(
        &session,
        &AccessRequest::new(Action::Input, terminal("t1"), NOW),
    )?;
    let before = fixture.authority.validate(&session, NOW)?;

    // When the owner rotates the writer's credential at the epoch it read.
    let rotated = fixture
        .authority
        .rotate(&fixture.owner, &Rotation::new(*issued.id(), before, NOW))?;

    // Then the old bearer, its session and its queued write are all refused at the new
    // principal epoch, and the replacement authenticates as the same principal.
    let after = rotated.withdrawn().principal_epoch();
    assert_eq!((before.get(), after.get()), (1, 2));
    assert_ne!(rotated.issued().id(), issued.id());
    assert_eq!(rotated.withdrawn().principal().as_str(), "writer");
    let old_login = fixture
        .authority
        .authenticate(&local_admission()?, Presentation::new(issued.bearer(), NOW))
        .err()
        .ok_or("the replaced bearer still authenticates")?;
    let old_session = fixture
        .authority
        .validate(&session, NOW)
        .err()
        .ok_or("the replaced session is still valid")?;
    let commit = fixture
        .authority
        .recheck(
            &queued,
            &CommitCheck::new(Action::Input, terminal("t1"), NOW),
        )
        .err()
        .ok_or("the queued write still commits")?;
    for refused in [&old_login, &old_session, &commit] {
        assert_eq!(refused.kind(), "credential_revoked");
        assert_eq!(refused.principal_epoch(), Some(after));
    }
    let fresh = fixture.authority.authenticate(
        &local_admission()?,
        Presentation::new(rotated.issued().bearer(), NOW),
    )?;
    assert_eq!(fresh.principal().as_str(), "writer");
    assert_eq!(fixture.authority.validate(&fresh, NOW)?, after);
    Ok(())
}

#[test]
fn rotation_keeps_each_permission_separate() -> TestResult {
    // Given a principal that may answer prompts but neither type nor take over.
    let fixture = Fixture::new()?;
    let (_, issued) = fixture.session(
        "reviewer",
        vec![grant(Permission::PromptAnswer, ScopeResource::Machine)],
    )?;
    let rotation = Rotation::new(*issued.id(), PrincipalEpoch::new(1), NOW);

    // When it is rotated.
    let rotated = fixture.authority.rotate(&fixture.owner, &rotation)?;
    let session = fixture.authority.authenticate(
        &local_admission()?,
        Presentation::new(rotated.issued().bearer(), NOW),
    )?;

    // Then the replacement answers prompts and still may not write or take over.
    let outcomes = [Action::PromptAnswer, Action::Input, Action::Takeover]
        .map(|action| fixture.outcome(&session, &AccessRequest::new(action, terminal("t1"), NOW)));
    assert_eq!(outcomes, ["allowed", "forbidden", "forbidden"]);
    Ok(())
}

#[test]
fn a_stale_principal_epoch_or_a_dead_credential_refuses_rotation_and_changes_nothing() -> TestResult
{
    // Given a writer already rotated once and a second writer credential revoked.
    let fixture = Fixture::new()?;
    let (_, first) = fixture.session("writer", writer_grants())?;
    let rotated = fixture.authority.rotate(
        &fixture.owner,
        &Rotation::new(*first.id(), PrincipalEpoch::new(1), NOW),
    )?;
    let (_, gone) = fixture.session("other", writer_grants())?;
    fixture.revoke(*gone.id())?;

    // When a second administrator rotates with the epoch it read before, the replaced
    // credential is rotated again, or an expiry outlives the owner.
    let stale = fixture.authority.rotate(
        &fixture.owner,
        &Rotation::new(*rotated.issued().id(), PrincipalEpoch::new(1), NOW),
    );
    let again = fixture.authority.rotate(
        &fixture.owner,
        &Rotation::new(*first.id(), PrincipalEpoch::new(2), NOW),
    );
    let revoked = fixture.authority.rotate(
        &fixture.owner,
        &Rotation::new(*gone.id(), PrincipalEpoch::new(2), NOW),
    );
    let beyond = fixture.authority.rotate(
        &fixture.owner,
        &Rotation::new(*rotated.issued().id(), PrincipalEpoch::new(2), NOW)
            .until(TimestampMs::new(OWNER_EXPIRY.get().saturating_add(1))),
    );

    // Then each is refused with its own kind and the current replacement still works.
    let stale = stale.err().ok_or("a stale epoch rotated")?;
    assert_eq!(stale.kind(), "version_conflict");
    assert_eq!(stale.principal_epoch(), Some(PrincipalEpoch::new(2)));
    assert_eq!(
        again.err().map(|error| error.kind()),
        Some("credential_revoked")
    );
    assert_eq!(
        revoked.err().map(|error| error.kind()),
        Some("credential_revoked")
    );
    assert_eq!(
        beyond.err().map(|error| error.kind()),
        Some("grant_exceeds_issuer")
    );
    let current = fixture.authority.authenticate(
        &local_admission()?,
        Presentation::new(rotated.issued().bearer(), NOW),
    )?;
    assert_eq!(
        fixture.authority.validate(&current, NOW)?,
        PrincipalEpoch::new(2)
    );
    Ok(())
}

#[test]
fn only_an_administrator_rotates_or_withdraws_and_expiry_needs_no_admin() -> TestResult {
    // Given a writer that tries to rotate and withdraw its own credential.
    let fixture = Fixture::new()?;
    let (session, issued) = fixture.session("writer", writer_grants())?;

    // When it asks, and when its credential later expires.
    let rotate = fixture.authority.rotate(
        &session,
        &Rotation::new(*issued.id(), PrincipalEpoch::new(1), NOW),
    );
    let withdraw = fixture
        .authority
        .withdraw(&session, &Revocation::new(*issued.id(), NOW));
    let expired = fixture.authority.validate(&session, ISSUED_EXPIRY);

    // Then both are forbidden and expiry refuses the session without moving the epoch.
    assert_eq!(rotate.err().map(|error| error.kind()), Some("forbidden"));
    assert_eq!(withdraw.err().map(|error| error.kind()), Some("forbidden"));
    assert_eq!(
        expired.err().map(|error| error.kind()),
        Some("credential_expired")
    );
    let withdrawn = fixture
        .authority
        .withdraw(&fixture.owner, &Revocation::new(*issued.id(), NOW))?;
    assert_eq!(withdrawn.principal_epoch(), PrincipalEpoch::new(2));
    let repeated = fixture
        .authority
        .withdraw(&fixture.owner, &Revocation::new(*issued.id(), NOW))?;
    assert_eq!(repeated.principal_epoch(), PrincipalEpoch::new(3));
    Ok(())
}

#[test]
fn a_bearer_with_a_live_id_and_a_wrong_secret_is_not_a_credential() -> TestResult {
    // Given a live credential.
    let fixture = Fixture::new()?;
    let (_, issued) = fixture.session("writer", writer_grants())?;
    let bearer = issued.bearer();
    let (prefix, secret) = bearer.rsplit_once('.').ok_or("bearer shape")?;

    // When the same id is presented with the secret's last digit changed.
    let flipped = if secret.ends_with('0') { '1' } else { '0' };
    let forged = format!("{prefix}.{}{flipped}", &secret[..secret.len() - 1]);
    let refused = fixture
        .authority
        .authenticate(&local_admission()?, Presentation::new(&forged, NOW));

    // Then it is refused as not issued, revealing nothing about the live credential.
    assert_eq!(
        refused.err().map(|error| error.kind()),
        Some("invalid_credential")
    );
    Ok(())
}

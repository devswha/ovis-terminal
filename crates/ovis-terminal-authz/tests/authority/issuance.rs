use ovis_terminal_authz::{
    AccessRequest, Action, AuthzError, Issuance, IssueRequest, Permission, Presentation,
    PrincipalId, ScopeResource, TargetResource,
};
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use proptest::prelude::{ProptestConfig, any, prop_assert_eq, proptest};
use serde_json::{Value, json};

use super::{Fixture, NOW, TestResult, WireIds, grant, local_admission, target, terminal_scope};

fn request_json(expires: &Value, grants: &Value) -> String {
    json!({ "principal": "agent", "expires_at_ms": expires, "grants": grants }).to_string()
}

fn observe_grant() -> Value {
    json!([{ "permission": "observe", "scope": { "machine": "m1", "resource": { "kind": "terminal", "id": "t1" } } }])
}

fn rejection(
    result: Result<IssueRequest<WireIds>, AuthzError>,
) -> TestResult<(&'static str, String)> {
    let error = result.err().ok_or("forged request was accepted")?;
    Ok((
        error.kind(),
        error.field_path().unwrap_or_default().to_owned(),
    ))
}

#[test]
fn issued_credential_reports_counters_as_decimal_strings() -> TestResult {
    let fixture = Fixture::new()?;
    let issued = fixture.issue(
        "agent",
        vec![grant(Permission::Observe, ScopeResource::Machine)],
    )?;

    let value = serde_json::to_value(&issued)?;

    assert_eq!(
        value.pointer("/epoch"),
        Some(&json!(issued.epoch().get().to_string()))
    );
    assert_eq!(value.pointer("/expires_at_ms"), Some(&json!("5000000")));
    assert_eq!(
        value.pointer("/grants/0/scope/resource"),
        Some(&json!({ "kind": "machine" }))
    );
    Ok(())
}

#[test]
fn numeric_counter_is_rejected_with_its_field_path() -> TestResult {
    let numeric = IssueRequest::from_json(&request_json(&json!(5_000_000_u64), &observe_grant()));

    assert_eq!(
        rejection(numeric)?,
        ("invalid_params", "expires_at_ms".to_owned())
    );
    Ok(())
}

#[test]
fn forged_scope_documents_are_rejected_with_field_paths() -> TestResult {
    let expires = json!("5000000");
    let unknown_permission = json!([{ "permission": "superuser", "scope": { "machine": "m1", "resource": { "kind": "machine" } } }]);
    let unknown_resource = json!([{ "permission": "observe", "scope": { "machine": "m1", "resource": { "kind": "everything" } } }]);
    let smuggled_field = json!([{ "permission": "observe", "scope": { "machine": "m1", "resource": { "kind": "terminal", "id": "t1", "wildcard": true } } }]);
    let mut extra_root: Value = serde_json::from_str(&request_json(&expires, &observe_grant()))?;
    extra_root
        .as_object_mut()
        .ok_or("request is not an object")?
        .insert("admin".to_owned(), json!(true));

    let outcomes = [
        rejection(IssueRequest::from_json(&request_json(
            &expires,
            &unknown_permission,
        )))?,
        rejection(IssueRequest::from_json(&request_json(
            &expires,
            &unknown_resource,
        )))?,
        rejection(IssueRequest::from_json(&request_json(
            &expires,
            &smuggled_field,
        )))?,
        rejection(IssueRequest::from_json(&extra_root.to_string()))?,
        rejection(IssueRequest::from_json(
            &request_json(&expires, &json!([])).replace("agent", ""),
        ))?,
    ];

    let kinds = outcomes.iter().map(|(kind, _)| *kind).collect::<Vec<_>>();
    assert_eq!(kinds, ["invalid_params"; 5]);
    let paths = outcomes
        .iter()
        .map(|(_, path)| path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            "grants[0].permission",
            "grants[0].scope.resource.kind",
            "grants[0].scope.resource",
            "admin",
            "principal"
        ]
    );
    Ok(())
}

#[test]
fn payload_from_an_unprivileged_principal_grants_nothing() -> TestResult {
    let fixture = Fixture::new()?;
    let (agent, _) = fixture.session(
        "agent-t1",
        vec![grant(Permission::Interact, terminal_scope("t1"))],
    )?;
    let injected = json!([{ "permission": "admin", "scope": { "machine": "m1", "resource": { "kind": "machine" } } }]);
    let request = IssueRequest::from_json(&request_json(&json!("5000000"), &injected))?;

    let escalation = fixture.authority.issue(&agent, Issuance::new(request, NOW));
    let administer = fixture.authority.authorize(
        &agent,
        &AccessRequest::new(Action::Administer, target(TargetResource::Machine), NOW),
    );

    assert_eq!(
        escalation.err().ok_or("escalation was issued")?.kind(),
        "forbidden"
    );
    assert_eq!(
        administer.err().ok_or("payload granted admin")?.kind(),
        "forbidden"
    );
    Ok(())
}

#[test]
fn issuer_cannot_grant_beyond_its_own_authority() -> TestResult {
    let fixture = Fixture::new()?;
    let (admin, _) = fixture.session(
        "admin",
        vec![grant(Permission::Admin, ScopeResource::Machine)],
    )?;
    let interact = vec![grant(Permission::Interact, ScopeResource::Machine)];
    let other_machine = vec![ovis_terminal_authz::Grant::new(
        Permission::Observe,
        ovis_terminal_authz::ResourceScope::new(
            MachineId::new("m2".to_owned()),
            ScopeResource::Machine,
        ),
    )];
    let principal = PrincipalId::parse("delegate")?;

    let unheld = fixture.authority.issue(
        &admin,
        Issuance::new(
            IssueRequest::new(
                principal.clone(),
                interact.clone(),
                TimestampMs::new(2_000_000),
            ),
            NOW,
        ),
    );
    let foreign = fixture.authority.issue(
        &fixture.owner,
        Issuance::new(
            IssueRequest::new(
                principal.clone(),
                other_machine,
                TimestampMs::new(2_000_000),
            ),
            NOW,
        ),
    );
    let outlives_issuer = fixture.authority.issue(
        &fixture.owner,
        Issuance::new(
            IssueRequest::new(
                principal.clone(),
                interact.clone(),
                TimestampMs::new(u64::MAX),
            ),
            NOW,
        ),
    );
    let already_expired = fixture.authority.issue(
        &fixture.owner,
        Issuance::new(IssueRequest::new(principal, interact, NOW), NOW),
    );

    let kinds = [unheld, foreign, outlives_issuer, already_expired]
        .map(|result| result.err().map(|error| error.kind()));
    assert_eq!(
        kinds,
        [
            Some("grant_exceeds_issuer"),
            Some("grant_exceeds_issuer"),
            Some("grant_exceeds_issuer"),
            Some("invalid_params")
        ]
    );
    Ok(())
}

#[test]
fn forged_bearer_tokens_are_rejected() -> TestResult {
    let fixture = Fixture::new()?;
    let issued = fixture.issue(
        "agent",
        vec![grant(Permission::Observe, ScopeResource::Machine)],
    )?;
    let (prefix, secret) = issued
        .bearer()
        .rsplit_once('.')
        .ok_or("bearer has no secret")?;
    let flipped = format!("{prefix}.{}", secret.chars().rev().collect::<String>());
    let unknown_id = format!("ovt1.c999.{secret}");
    let truncated = format!(
        "{prefix}.{}",
        secret.get(..secret.len() - 2).ok_or("short secret")?
    );
    let uppercase = format!("{prefix}.{}", secret.to_uppercase());
    let admission = local_admission()?;

    let kinds = [
        "",
        "garbage",
        flipped.as_str(),
        unknown_id.as_str(),
        truncated.as_str(),
        uppercase.as_str(),
    ]
    .map(|bearer| {
        fixture
            .authority
            .authenticate(&admission, Presentation::new(bearer, NOW))
            .err()
            .map(|error| error.kind())
    });

    assert_eq!(kinds[0], Some("unauthenticated"));
    assert_eq!(kinds[1..], [Some("invalid_credential"); 5]);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn expiry_round_trips_every_u64_as_a_decimal_string(expires in any::<u64>()) {
        let text = request_json(&json!(expires.to_string()), &observe_grant());

        let parsed = IssueRequest::<WireIds>::from_json(&text).map_err(|error| error.to_string());

        prop_assert_eq!(parsed.map(|request| request.expires_at().get()), Ok(expires));
    }
}

#[test]
fn expiry_preserves_u64_max_exactly() -> TestResult {
    let text = request_json(&json!(u64::MAX.to_string()), &observe_grant());

    let parsed = IssueRequest::<WireIds>::from_json(&text)?;

    assert_eq!(parsed.expires_at(), TimestampMs::new(u64::MAX));
    assert_eq!(
        serde_json::to_value(parsed.expires_at())?,
        json!("18446744073709551615")
    );
    Ok(())
}

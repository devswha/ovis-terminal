use ovis_terminal_authz::{AccessRequest, Action, Permission, PrincipalId, TargetResource};
use ovis_terminal_protocol::errors::OpId;

use super::{Fixture, NOW, TestResult, grant, target, terminal_scope};

#[test]
fn own_operations_remain_inspectable_under_resource_scoped_credentials() -> TestResult {
    // Given a principal with grants confined to one terminal.
    let fixture = Fixture::new()?;
    let (session, issued) = fixture.session(
        "agent",
        vec![
            grant(Permission::Observe, terminal_scope("one")),
            grant(Permission::Interact, terminal_scope("one")),
        ],
    )?;
    let operation = target(TargetResource::Operation {
        principal: PrincipalId::parse("agent")?,
        op_id: OpId::new("01800000-0000-7000-8000-000000000001".into()),
    });
    let read = AccessRequest::new(Action::InspectOperation, operation.clone(), NOW);
    let cancel = AccessRequest::new(Action::CancelOperation, operation, NOW);
    // When operation access is decided and then revoked before cancellation commits.
    let read_decision = fixture.authority.authorize(&session, &read)?;
    let cancel_decision = fixture.authority.authorize(&session, &cancel)?;
    fixture.authority.recheck(&read_decision, &read)?;
    fixture.revoke(*issued.id())?;
    // Then ownership permits inspection, but revocation still fences cancellation.
    assert_eq!(
        fixture
            .authority
            .recheck(&cancel_decision, &cancel)
            .expect_err("revocation fences mutation")
            .kind(),
        "credential_revoked"
    );
    Ok(())
}

#[test]
fn operation_ownership_cannot_authorize_other_principals_or_domain_effects() -> TestResult {
    // Given a principal whose ordinary grants are resource-scoped.
    let fixture = Fixture::new()?;
    let (session, _) = fixture.session(
        "agent",
        vec![
            grant(Permission::Observe, terminal_scope("one")),
            grant(Permission::Interact, terminal_scope("one")),
        ],
    )?;
    // When it requests somebody else's receipt, or a domain effect on an operation ID.
    let foreign = target(TargetResource::Operation {
        principal: PrincipalId::parse("other")?,
        op_id: OpId::new("01800000-0000-7000-8000-000000000001".into()),
    });
    let own = target(TargetResource::Operation {
        principal: PrincipalId::parse("agent")?,
        op_id: OpId::new("01800000-0000-7000-8000-000000000001".into()),
    });
    // Then both requests are refused, without broadening terminal authority.
    for request in [
        AccessRequest::new(Action::InspectOperation, foreign, NOW),
        AccessRequest::new(Action::Launch, own, NOW),
        AccessRequest::new(Action::Input, super::terminal("two"), NOW),
    ] {
        assert_eq!(fixture.outcome(&session, &request), "forbidden");
    }
    Ok(())
}

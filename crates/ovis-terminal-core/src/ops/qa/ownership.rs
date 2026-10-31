use ovis_terminal_authz::PrincipalId;
use ovis_terminal_core::ops::{Context, Operations};
use ovis_terminal_protocol::schema::scalar::TimestampMs;

use super::{Fixture, TestResult, context, identity, request};

#[test]
fn owner_resolution_survives_reopen_and_preserves_principal_scoped_ids() -> TestResult {
    // Given an operation accepted for the original principal in real SQLite.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let original = Operations::new(&core, context(1)?);
    let req = request("one")?;
    assert_eq!(original.owner(req.op_id())?, None);
    original.accept(&req)?;
    core.close()?;
    // When a different principal resolves that ID after reopening the same store.
    let core = fixture.open(1)?;
    let other_id = PrincipalId::parse("other")?;
    let other = Operations::new(
        &core,
        Context::new(
            other_id.clone(),
            identity(1)?,
            TimestampMs::new(100_000_000),
        ),
    );
    assert_eq!(
        other.owner(req.op_id())?,
        Some(PrincipalId::parse("operator")?)
    );
    // Then lookup does not grant access, and independent reuse still resolves to the caller.
    assert_eq!(
        other
            .get(req.op_id())
            .expect_err("principal-scoped lookup")
            .kind(),
        "not_found"
    );
    other.accept(&req)?;
    assert_eq!(other.owner(req.op_id())?, Some(other_id));
    assert_eq!(
        Operations::new(&core, context(1)?).owner(req.op_id())?,
        Some(PrincipalId::parse("operator")?)
    );
    core.close()?;
    Ok(())
}

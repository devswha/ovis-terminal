//! Adversarial operation state transitions.

use super::{
    CancelOutcome, Context, Fixture, OpState, OpenEnum, Operations, PrincipalId, Request,
    TestResult, TimestampMs, context, identity, json, request,
};

#[test]
fn cancelling_acceptance_prevents_the_effect_after_reopen() -> TestResult {
    // Given an accepted operation that has not crossed the external-effect boundary.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let req = request("cancel")?;
    operations.accept(&req)?;

    // When cancellation is durably acknowledged and the core reopens.
    let cancelled = operations.cancel(req.op_id())?;
    assert_eq!(
        cancelled.outcome,
        OpenEnum::Known(CancelOutcome::CancelledBeforeCommit)
    );
    core.close()?;
    let core = fixture.open(2)?;
    let operations = Operations::new(&core, context(2)?);

    // Then a retry cannot obtain permission to perform the cancelled effect.
    assert_eq!(
        operations.accept(&req)?.state,
        OpenEnum::Known(OpState::Cancelled)
    );
    assert_eq!(
        operations.start(req.op_id()).expect_err("cancelled").kind(),
        "cancelled"
    );
    core.close()?;
    Ok(())
}

#[test]
fn failure_and_completion_remain_queryable_and_cancellation_never_undoes_them() -> TestResult {
    use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
    // Given a validation failure before any effect.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let req = request("bad-program")?;
    operations.accept(&req)?;
    let error = BoundaryError::domain(ErrorKind::from("spawn_failed".to_owned()), false)
        .with_op_id(req.op_id().clone());
    let failed = operations.fail(req.op_id(), error.data().clone())?;
    assert_eq!(failed.state, OpenEnum::Known(OpState::Failed));
    core.close()?;

    // When cancellation and a retry occur after reopen.
    let core = fixture.open(2)?;
    let operations = Operations::new(&core, context(2)?);
    let cancelled = operations.cancel(req.op_id())?;

    // Then the original typed failure remains inspectable and cannot start again.
    assert_eq!(
        cancelled.outcome,
        OpenEnum::Known(CancelOutcome::CancelledBeforeCommit)
    );
    assert_eq!(cancelled.operation, failed);
    assert_eq!(operations.accept(&req)?, failed);
    assert_eq!(
        failed.error.as_ref().map(|error| error.kind.token()),
        Some("spawn_failed")
    );
    assert!(operations.start(req.op_id()).is_err());
    core.close()?;
    Ok(())
}

#[test]
fn running_failure_is_durable_and_cancellation_reports_no_effect() -> TestResult {
    use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};

    // Given an operation whose intent committed before a holder rejected the work.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let req = request("missing-program")?;
    operations.accept(&req)?;
    let permit = operations.start(req.op_id())?;
    let error = BoundaryError::domain(ErrorKind::from("spawn_failed".to_owned()), false)
        .with_op_id(req.op_id().clone());

    // When the proven no-effect failure is committed and the core reopens.
    let failed = operations.fail_running(permit, error.data().clone())?;
    assert_eq!(failed.state, OpenEnum::Known(OpState::Failed));
    core.close()?;
    let core = fixture.open(2)?;
    let operations = Operations::new(&core, context(2)?);
    let cancellation = operations.cancel(req.op_id())?;

    // Then failure facts remain queryable and cancellation claims only that no effect began.
    assert_eq!(
        cancellation.outcome,
        OpenEnum::Known(CancelOutcome::CancelledBeforeCommit)
    );
    assert_eq!(cancellation.operation, failed);
    assert_eq!(operations.accept(&req)?, failed);
    assert_eq!(
        failed.error.as_ref().map(|error| error.kind.token()),
        Some("spawn_failed")
    );
    assert!(operations.start(req.op_id()).is_err());
    core.close()?;
    Ok(())
}

#[test]
fn running_cancel_is_pending_and_completion_retains_its_receipt() -> TestResult {
    // Given an operation whose intent already committed.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let req = request("running")?;
    operations.accept(&req)?;
    let permit = operations.start(req.op_id())?;

    // When cancellation races work that can still finish.
    let pending = operations.cancel(req.op_id())?;
    assert_eq!(pending.outcome, OpenEnum::Known(CancelOutcome::Pending));
    let committed = operations.commit(permit, json!({"finished":true}))?;

    // Then cancellation never retracts the completion or grants another effect.
    let repeated = operations.cancel(req.op_id())?;
    assert_eq!(
        repeated.outcome,
        OpenEnum::Known(CancelOutcome::AlreadyCommitted)
    );
    assert_eq!(repeated.operation, committed);
    assert!(
        committed
            .evidence
            .is_some_and(|evidence| evidence.cancellation_requested)
    );
    assert!(operations.start(req.op_id()).is_err());
    core.close()?;
    Ok(())
}

#[test]
fn canonical_params_reuse_but_arrays_and_methods_cannot_change() -> TestResult {
    // Given an acceptance with nested object parameters.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let first = request("same")?;
    let accepted = operations.accept(&first)?;

    // When the object key order changes, but then an array or method changes.
    let reordered = Request::new(
        first.op_id().clone(),
        "terminal.run".into(),
        serde_json::from_str(r#"{"env":{"A":"1","B":"2"},"argv":["same"]}"#)?,
    )?;
    assert_eq!(operations.accept(&reordered)?, accepted);
    let array = Request::new(
        first.op_id().clone(),
        "terminal.run".into(),
        json!({"argv":["different"]}),
    )?;
    let method = Request::new(
        first.op_id().clone(),
        "terminal.kill".into(),
        json!({"argv":["same"]}),
    )?;

    // Then only equivalent canonical work is reused.
    assert_eq!(
        operations
            .accept(&array)
            .expect_err("different array")
            .kind(),
        "op_reused"
    );
    let rejected = operations.accept(&method).expect_err("different method");
    assert_eq!(rejected.kind(), "op_reused");
    assert_eq!(rejected.receipt(), Some(&accepted));
    core.close()?;
    Ok(())
}

#[test]
fn principal_scope_isolated_and_retention_expiration_prevents_reexecution() -> TestResult {
    // Given an operation accepted by one principal.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let req = request("scoped")?;
    operations.accept(&req)?;
    let other = Context::new(
        PrincipalId::parse("other")?,
        identity(1)?,
        TimestampMs::new(100_000_000),
    );
    let other_operations = Operations::new(&core, other);
    assert_eq!(
        other_operations
            .get(req.op_id())
            .expect_err("private")
            .kind(),
        "not_found"
    );
    assert_eq!(
        other_operations.accept(&req)?.state,
        OpenEnum::Known(OpState::Accepted)
    );

    // When the 24-hour advertised acceptance window elapses.
    let expired = Operations::new(&core, context(1)?.at(TimestampMs::new(186_400_000)));

    // Then even a retained ID cannot execute or be accepted as new.
    assert_eq!(
        expired.get(req.op_id()).expect_err("expired query").kind(),
        "op_expired"
    );
    assert_eq!(
        expired.accept(&req).expect_err("expired mutation").kind(),
        "op_expired"
    );
    assert_eq!(
        expired
            .start(req.op_id())
            .expect_err("expired start")
            .kind(),
        "op_expired"
    );
    core.close()?;
    Ok(())
}

#[test]
fn acceptance_floor_survives_pruning_reopen_and_clock_rollback() -> TestResult {
    // Given a receipt that ages out before a newer operation is admitted.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let old = request("old")?;
    Operations::new(&core, context(1)?).accept(&old)?;
    let later = Operations::new(&core, context(1)?.at(TimestampMs::new(186_400_001)));
    let uuid =
        uuid::Uuid::from_u128((186_400_001_u128 << 80) | (7_u128 << 76) | (2_u128 << 62) | 1);
    let fresh = Request::new(
        super::OpId::new(uuid.to_string()),
        "terminal.run".into(),
        json!({"argv":["new"]}),
    )?;
    later.accept(&fresh)?;
    core.close()?;

    // When the clock moves backward after reopening the pruned ledger.
    let core = fixture.open(2)?;
    let earlier = Operations::new(&core, context(2)?);

    // Then the persisted acceptance floor still prevents old work from executing anew.
    assert_eq!(
        earlier
            .accept(&old)
            .expect_err("expired despite rollback")
            .kind(),
        "op_expired"
    );
    assert_eq!(
        earlier.get(old.op_id()).expect_err("pruned receipt").kind(),
        "op_expired"
    );
    core.close()?;
    Ok(())
}

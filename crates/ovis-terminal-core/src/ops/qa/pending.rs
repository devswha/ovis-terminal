//! Cancellation that reaches a request before its durable accept.

use super::{
    CancelOutcome, Fixture, OpState, OpenEnum, Operations, TestResult, context, json, request,
};

#[test]
fn pre_accept_cancel_commits_acceptance_and_cancellation_together_and_survives_restart()
-> TestResult {
    // Given a request that no ledger row names yet, observed from the current cursor.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let req = request("pending")?;
    let observer = core.subscribe(None)?;
    let before = observer.snapshot.ok_or("snapshot")?.cursor.seq.get();

    // When its cancellation is delivered as the accept commits.
    let cancelled = operations.accept_cancelled(&req)?;

    // Then one unit publishes accepted then cancelled, and the record can never start.
    assert_eq!(
        cancelled.outcome,
        OpenEnum::Known(CancelOutcome::CancelPending)
    );
    assert_eq!(
        cancelled.operation.state,
        OpenEnum::Known(OpState::Cancelled)
    );
    let accepted = observer.delivery.try_recv()?;
    let settled = observer.delivery.try_recv()?;
    assert_eq!(accepted.event_type, "op.accepted");
    assert_eq!(settled.event_type, "op.cancelled");
    assert_eq!(accepted.cursor.seq.get(), before + 1);
    assert_eq!(settled.cursor.seq.get(), before + 2);
    assert_eq!(cancelled.operation.updated_seq.get(), before + 2);
    assert_eq!(operations.get(req.op_id())?, cancelled.operation);
    assert_eq!(
        operations.start(req.op_id()).expect_err("cancelled").kind(),
        "cancelled"
    );
    core.close()?;

    // And after a restart, replaying the same request still observes the cancellation.
    let core = fixture.open(2)?;
    let operations = Operations::new(&core, context(2)?);
    assert_eq!(operations.accept(&req)?, cancelled.operation);
    assert_eq!(
        operations.start(req.op_id()).expect_err("replayed").kind(),
        "cancelled"
    );
    let again = operations.cancel(req.op_id())?;
    assert_eq!(
        again.outcome,
        OpenEnum::Known(CancelOutcome::CancelledBeforeCommit)
    );
    assert_eq!(again.operation, cancelled.operation);
    core.close()?;
    Ok(())
}

#[test]
fn late_delivery_to_a_retained_identity_reports_what_was_actually_cancelled() -> TestResult {
    // Given one identity accepted but not started, and one already committed.
    let fixture = Fixture::new();
    let core = fixture.open(1)?;
    let operations = Operations::new(&core, context(1)?);
    let waiting = request("waiting")?;
    operations.accept(&waiting)?;
    let done = super::Request::new(
        super::OpId::new("000005f5-e100-7000-8000-000000000002".into()),
        "terminal.run".into(),
        json!({"argv":["done"]}),
    )?;
    operations.accept(&done)?;
    let permit = operations.start(done.op_id())?;
    let receipt = operations.commit(permit, json!({"finished":true}))?;

    // When a pre-accept cancellation is delivered to each retained identity.
    let before = operations.accept_cancelled(&waiting)?;
    let after = operations.accept_cancelled(&done)?;

    // Then neither claims a pending delivery: one is cancelled, one keeps its receipt.
    assert_eq!(
        before.outcome,
        OpenEnum::Known(CancelOutcome::CancelledBeforeCommit)
    );
    assert_eq!(before.operation.state, OpenEnum::Known(OpState::Cancelled));
    assert_eq!(
        after.outcome,
        OpenEnum::Known(CancelOutcome::AlreadyCommitted)
    );
    assert_eq!(after.operation, receipt);
    let changed = request("changed")?;
    assert_eq!(
        operations
            .accept_cancelled(&changed)
            .expect_err("different work")
            .kind(),
        "op_reused"
    );
    core.close()?;
    Ok(())
}

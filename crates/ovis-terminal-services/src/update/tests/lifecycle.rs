use std::sync::atomic::Ordering;

use ovis_terminal_protocol::domain::update::{ProbeOutcome, RollbackReason};
use ovis_terminal_protocol::schema::methods::update::UpdateState;
use ovis_terminal_protocol::schema::scalar::{Incarnation, OpenEnum};

use super::fixture::{
    Fixture, LIVE, TestResult, release, sha256, stage_params, step_params, version,
};
use crate::update::{Interrupt, UpdateError};

#[test]
fn a_verified_build_is_selected_only_after_readiness_and_restart() -> TestResult {
    let fixture = Fixture::new()?;
    let updates = fixture.open()?;
    let original = updates.query()?.active;
    let digest = fixture.publish(("2.0.0", &super::host()), &release("2.0.0", 0))?;
    let staged = updates.stage(&stage_params("2.0.0", &digest)?, &Interrupt::never())?;
    assert_eq!(staged.state, OpenEnum::Known(UpdateState::Staged));
    assert_eq!(updates.query()?.active, original);
    let verified = updates.verify(&step_params(&staged.update_id)?)?;
    let build = verified.build.ok_or("verified without build facts")?;
    let activated = {
        let params = step_params(&staged.update_id)?;
        fixture.step(|| updates.activate(&params))
    }?;
    assert_eq!(activated.state, OpenEnum::Known(UpdateState::Activated));
    let receipt = activated.activation.ok_or("no activation receipt")?;
    assert_eq!(receipt.previous, original);
    assert_eq!(
        receipt.readiness.outcome,
        OpenEnum::Known(ProbeOutcome::Ready)
    );
    assert_eq!(
        receipt.restart.preserved_incarnations,
        LIVE.map(Incarnation::new)
    );
    assert_eq!(receipt.restart.ended_incarnations, []);
    let query = updates.query()?;
    assert_eq!(query.active_version, version("2.0.0")?);
    assert_eq!(query.active.binary_sha256, build.binary_sha256);
    assert_eq!(query.previous, Some(original));
    assert_eq!(query.live_incarnations, LIVE.map(Incarnation::new));
    assert_eq!(fixture.control.restarts.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn a_build_failing_readiness_is_rolled_back_with_its_evidence() -> TestResult {
    let fixture = Fixture::new()?;
    let updates = fixture.open()?;
    let original = updates.query()?.active;
    let digest = fixture.publish(("3.0.0", &super::host()), &release("3.0.0", 1))?;
    let staged = updates.stage(&stage_params("3.0.0", &digest)?, &Interrupt::never())?;
    updates.verify(&step_params(&staged.update_id)?)?;
    let refused = {
        let params = step_params(&staged.update_id)?;
        fixture.step(|| updates.activate(&params))
    };
    let Err(error) = refused else {
        return Err("a failing build was activated".into());
    };
    assert_eq!(error.kind(), "activation_failed");
    let query = updates.query()?;
    assert_eq!(query.active, original);
    assert_eq!(
        query.active.binary_sha256.as_str(),
        sha256(&std::fs::read(fixture.current_binary())?)
    );
    assert_eq!(query.pending, None);
    let record = query.updates.first().ok_or("record missing")?;
    assert_eq!(record.state, OpenEnum::Known(UpdateState::RolledBack));
    let rollback = record.rollback.clone().ok_or("no rollback receipt")?;
    assert_eq!(
        rollback.reason,
        OpenEnum::Known(RollbackReason::ReadinessFailed)
    );
    assert_eq!(rollback.restored, original);
    let readiness = rollback.readiness.ok_or("no readiness evidence")?;
    assert_eq!(readiness.outcome, OpenEnum::Known(ProbeOutcome::Exited));
    assert_eq!(readiness.exit_code, Some(1));
    assert!(readiness.stdout.contains("3.0.0"));
    assert_eq!(fixture.control.restarts.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn a_refused_restart_rolls_the_selection_back() -> TestResult {
    let fixture = Fixture::new()?;
    let updates = fixture.open()?;
    let original = updates.query()?.active;
    let digest = fixture.publish(("2.0.0", &super::host()), &release("2.0.0", 0))?;
    let staged = updates.stage(&stage_params("2.0.0", &digest)?, &Interrupt::never())?;
    updates.verify(&step_params(&staged.update_id)?)?;
    *fixture.control.refuse.lock().map_err(|_| "poisoned")? = true;
    let refused = {
        let params = step_params(&staged.update_id)?;
        fixture.step(|| updates.activate(&params))
    };
    assert!(matches!(
        refused,
        Err(UpdateError::ActivationFailed(RollbackReason::RestartFailed))
    ));
    assert_eq!(updates.query()?.active, original);
    Ok(())
}

#[test]
fn rollback_restores_the_replaced_build_through_a_restart() -> TestResult {
    let fixture = Fixture::new()?;
    let updates = fixture.open()?;
    let original = updates.query()?.active;
    let digest = fixture.publish(("2.0.0", &super::host()), &release("2.0.0", 0))?;
    let staged = updates.stage(&stage_params("2.0.0", &digest)?, &Interrupt::never())?;
    updates.verify(&step_params(&staged.update_id)?)?;
    {
        let params = step_params(&staged.update_id)?;
        fixture.step(|| updates.activate(&params))
    }?;
    let rolled = {
        let params = step_params(&staged.update_id)?;
        fixture.step(|| updates.rollback(&params))
    }?;
    assert_eq!(rolled.state, OpenEnum::Known(UpdateState::RolledBack));
    let receipt = rolled.rollback.ok_or("no rollback receipt")?;
    assert_eq!(receipt.reason, OpenEnum::Known(RollbackReason::Requested));
    assert_eq!(receipt.restored, original);
    assert!(receipt.restart.is_some());
    assert_eq!(updates.query()?.active, original);
    assert_eq!(fixture.control.restarts.load(Ordering::SeqCst), 2);
    let again = {
        let params = step_params(&staged.update_id)?;
        fixture.step(|| updates.rollback(&params))
    };
    assert_eq!(
        again.map_err(|error| error.kind()).err(),
        Some("invalid_update_state")
    );
    Ok(())
}

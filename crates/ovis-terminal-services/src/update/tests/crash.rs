//! A real activation stopped at a crash barrier, then reopened from the journal it wrote.

use std::sync::atomic::Ordering;

use ovis_terminal_protocol::domain::update::{ActivationPhase, RejectReason, RollbackReason};
use ovis_terminal_protocol::schema::methods::update::UpdateState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;

use super::fixture::{Fixture, TestResult, release, script, stage_params, step_params};
use crate::update::layout::load;
use crate::update::store::Journal;
use crate::update::{Interrupt, UpdateError, Updates};

/// Stages and verifies `2.0.0`, arms the barrier at `phase` and activates.
fn crashed(fixture: &Fixture, phase: ActivationPhase) -> TestResult<(Updates, Journal)> {
    let updates = fixture.open()?;
    let digest = fixture.publish(("2.0.0", &super::host()), &release("2.0.0", 0))?;
    let staged = updates.stage(&stage_params("2.0.0", &digest)?, &Interrupt::never())?;
    updates.verify(&step_params(&staged.update_id)?)?;
    *updates.crash_after.lock().map_err(|_| "poisoned")? = Some(phase);
    let params = step_params(&staged.update_id)?;
    let stopped = fixture.step(|| updates.activate(&params));
    assert!(
        matches!(stopped, Err(UpdateError::Io { .. })),
        "{stopped:?}"
    );
    let written = load::<Journal>(&fixture.install_root().join("activation.json"))?;
    Ok((updates, written.ok_or("activate left no journal")?))
}

#[test]
fn a_crash_after_the_ready_record_is_finished_on_open_without_a_second_restart() -> TestResult {
    let fixture = Fixture::new()?;
    let (updates, journal) = crashed(&fixture, ActivationPhase::Ready)?;
    assert_eq!(journal.phase, ActivationPhase::Ready);
    let restart = journal
        .restart
        .clone()
        .ok_or("ready journal without restart")?;
    drop(updates);
    let query = fixture.open()?.query()?;
    assert_eq!(query.active, journal.after.active.build);
    assert_eq!(query.pending, None);
    let record = query.updates.first().ok_or("record missing")?;
    assert_eq!(record.state, OpenEnum::Known(UpdateState::Activated));
    let receipt = record.activation.as_ref().ok_or("no activation receipt")?;
    assert_eq!(receipt.restart, restart);
    assert_eq!(receipt.op_id, journal.op_id);
    assert_eq!(fixture.control.restarts.load(Ordering::SeqCst), 1);
    assert!(!fixture.install_root().join("activation.json").exists());
    Ok(())
}

#[test]
fn a_crash_before_the_ready_record_is_rolled_back_on_open() -> TestResult {
    let fixture = Fixture::new()?;
    let (updates, journal) = crashed(&fixture, ActivationPhase::Selected)?;
    assert_eq!(journal.phase, ActivationPhase::Selected);
    assert_eq!(journal.restart, None);
    drop(updates);
    let query = fixture.open()?.query()?;
    assert_eq!(query.active, journal.before.active.build);
    let record = query.updates.first().ok_or("record missing")?;
    assert_eq!(record.state, OpenEnum::Known(UpdateState::RolledBack));
    let reason = record
        .rollback
        .as_ref()
        .map(|receipt| receipt.reason.clone());
    assert_eq!(reason, Some(OpenEnum::Known(RollbackReason::Interrupted)));
    assert!(!fixture.install_root().join("activation.json").exists());
    Ok(())
}

#[test]
fn an_executable_changed_after_verification_is_refused_before_selection() -> TestResult {
    let fixture = Fixture::new()?;
    let updates = fixture.open()?;
    let before = updates.query()?;
    let digest = fixture.publish(("2.0.0", &super::host()), &release("2.0.0", 0))?;
    let staged = updates.stage(&stage_params("2.0.0", &digest)?, &Interrupt::never())?;
    updates.verify(&step_params(&staged.update_id)?)?;
    let binary = fixture
        .install_root()
        .join("builds")
        .join(staged.update_id.as_str())
        .join("ovis-terminal");
    std::fs::write(&binary, script("2.0.0-tampered", 0))?;
    let params = step_params(&staged.update_id)?;
    let refused = fixture.step(|| updates.activate(&params));
    let Err(error) = refused else {
        return Err("a changed executable was activated".into());
    };
    assert!(matches!(
        error,
        UpdateError::Rejected(RejectReason::DigestMismatch)
    ));
    assert_eq!(error.kind(), "integrity_failed");
    let after = updates.query()?;
    assert_eq!(after.active, before.active);
    assert_eq!(after.previous, None);
    let record = after.updates.first().ok_or("record missing")?;
    assert_eq!(record.state, OpenEnum::Known(UpdateState::Rejected));
    let reason = record
        .rejection
        .as_ref()
        .map(|rejection| rejection.reason.clone());
    assert_eq!(reason, Some(OpenEnum::Known(RejectReason::DigestMismatch)));
    assert_eq!(fixture.control.restarts.load(Ordering::SeqCst), 0);
    assert!(!fixture.install_root().join("activation.json").exists());
    Ok(())
}

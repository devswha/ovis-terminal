use ovis_terminal_protocol::domain::update::{
    ActivationPhase, ActiveBuild, ProbeOutcome, ReadinessEvidence, RollbackReason,
};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::ServerBootId;
use ovis_terminal_protocol::schema::methods::server::ServerTransitionResult;
use ovis_terminal_protocol::schema::methods::update::{UpdateRecord, UpdateState};
use ovis_terminal_protocol::schema::scalar::OpenEnum;

use super::fixture::{Fixture, TestResult, release, stage_params, step_params};
use crate::update::Interrupt;
use crate::update::layout::{binary_name, store};
use crate::update::store::{Journal, Selected, Selection, Step};

#[test]
fn a_staged_update_survives_a_reopen_and_verifies_afterwards() -> TestResult {
    let fixture = Fixture::new()?;
    let digest = fixture.publish(("2.0.0", &super::host()), &release("2.0.0", 0))?;
    let staged = fixture
        .open()?
        .stage(&stage_params("2.0.0", &digest)?, &Interrupt::never())?;
    let reopened = fixture.open()?;
    assert_eq!(reopened.query()?.updates, vec![staged.clone()]);
    let verified = reopened.verify(&step_params(&staged.update_id)?)?;
    assert_eq!(verified.state, OpenEnum::Known(UpdateState::Verified));
    Ok(())
}

/// A verified update and the journal its activation writes before the swap.
fn interrupted(fixture: &Fixture, phase: ActivationPhase) -> TestResult<(Journal, UpdateRecord)> {
    let updates = fixture.open()?;
    let digest = fixture.publish(("2.0.0", &super::host()), &release("2.0.0", 0))?;
    let staged = updates.stage(&stage_params("2.0.0", &digest)?, &Interrupt::never())?;
    let record = updates.verify(&step_params(&staged.update_id)?)?;
    let build = record.build.clone().ok_or("no build facts")?;
    let before = updates.lock()?.selection.clone();
    let binary = updates
        .layout
        .build(&record.update_id)
        .join(binary_name(&record.target));
    let next = Selected {
        build: ActiveBuild::new(
            record.version.clone(),
            record.target.clone(),
            build.binary_sha256,
        ),
        binary,
        update_id: Some(record.update_id.clone()),
    };
    let after = before.advanced(next);
    let ready = phase == ActivationPhase::Ready;
    let journal = Journal {
        update_id: record.update_id.clone(),
        op_id: OpId::new("op-interrupted".to_owned()),
        step: Step::Activate,
        phase,
        before,
        after,
        readiness: ready
            .then(|| ReadinessEvidence::new(ProbeOutcome::Ready, (Some(0), String::new()))),
        restart: ready.then(|| {
            ServerTransitionResult::restarted(
                (
                    ServerBootId::new("b1".to_owned()),
                    ServerBootId::new("b2".to_owned()),
                ),
                Vec::new(),
                Vec::new(),
            )
        }),
    };
    store(&fixture.install_root().join("activation.json"), &journal)?;
    Ok((journal, record))
}

fn write_selection(fixture: &Fixture, selection: &Selection) -> TestResult {
    store(&fixture.install_root().join("active.json"), selection)?;
    Ok(())
}

#[test]
fn an_activation_interrupted_before_readiness_is_reverted_on_open() -> TestResult {
    for (phase, swapped) in [
        (ActivationPhase::Intent, false),
        (ActivationPhase::Selected, true),
        // Recovery itself was interrupted after selecting the old build again.
        (ActivationPhase::Selected, false),
    ] {
        let fixture = Fixture::new()?;
        let (journal, _) = interrupted(&fixture, phase)?;
        let on_disk = if swapped {
            &journal.after
        } else {
            &journal.before
        };
        write_selection(&fixture, on_disk)?;
        for _ in 0..2 {
            let query = fixture.open()?.query()?;
            assert_eq!(
                query.active, journal.before.active.build,
                "{phase:?} {swapped}"
            );
            assert_eq!(query.pending, None);
            let record = query.updates.first().ok_or("record missing")?;
            assert_eq!(record.state, OpenEnum::Known(UpdateState::RolledBack));
            let reason = record
                .rollback
                .as_ref()
                .map(|receipt| receipt.reason.clone());
            assert_eq!(reason, Some(OpenEnum::Known(RollbackReason::Interrupted)));
        }
        assert!(!fixture.install_root().join("activation.json").exists());
    }
    Ok(())
}

#[test]
fn an_activation_interrupted_after_restart_is_finished_on_open() -> TestResult {
    let fixture = Fixture::new()?;
    let (journal, _) = interrupted(&fixture, ActivationPhase::Ready)?;
    write_selection(&fixture, &journal.after)?;
    let query = fixture.open()?.query()?;
    assert_eq!(query.active, journal.after.active.build);
    let record = query.updates.first().ok_or("record missing")?;
    assert_eq!(record.state, OpenEnum::Known(UpdateState::Activated));
    assert!(record.activation.is_some());
    assert!(!fixture.install_root().join("activation.json").exists());
    Ok(())
}

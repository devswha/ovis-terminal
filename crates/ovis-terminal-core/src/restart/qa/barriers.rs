use ovis_terminal_protocol::domain::restart::{
    Delivery, InterruptedOp, InterruptedOutcome, RestartPhase,
};
use ovis_terminal_protocol::schema::methods::server::OpState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;

use super::*;
use crate::restart::RestartIntent;
use crate::restart::sequence::successor;

/// Persists the intent at `phase` (handing over when the phase lies past it), then the
/// control process dies: its link is released and no program is signalled.
fn crash_at(control: Control, phase: RestartPhase) -> TestResult<RestartIntent> {
    let intent = {
        let mut inner = control.lock()?;
        let mut intent = inner.intent(op())?;
        intent.phase = phase;
        if !phase.before_handover() {
            inner.hand_over()?;
        }
        inner.plan.intent().store(&intent)?;
        intent
    };
    control.close()?;
    Ok(intent)
}

#[test]
fn a_crash_at_any_barrier_is_finished_by_the_next_open_with_the_same_processes() -> TestResult {
    for phase in [
        RestartPhase::IntentPersisted,
        RestartPhase::Suspended,
        RestartPhase::Announced,
        RestartPhase::HandedOver,
        RestartPhase::Booted,
    ] {
        // Given: a live child and a restart that crashed after persisting `phase`.
        let fixture = Fixture::new()?;
        let (control, _) = fixture.open()?;
        let record = spawn(&control, fixture.launch("printf ARMED; read gate")?)?;
        marker(&control, (record.id(), b"ARMED"))?;
        let process = record.lifecycle.process().ok_or("process")?.clone();
        let intent = crash_at(control, phase)?;

        // When: the next control opens.
        let (control, resumed) = fixture.open()?;

        // Then: it serves the intent's boot in the same lifetime with the same process.
        let receipt = resumed.ok_or("the pending intent was not finished")?;
        assert!(receipt.resumed);
        let preserved: Vec<_> = receipt.preserved.iter().map(|held| &held.process).collect();
        assert_eq!(preserved, vec![&process], "{phase:?}");
        assert_eq!(receipt.lost, Vec::new(), "{phase:?}");
        assert_eq!(control.identity()?.server_boot_id(), intent.to);
        assert_eq!(
            control.identity()?.runtime_id(),
            fixture.identity.runtime_id()
        );
        assert_eq!(control.phase()?, RestartPhase::Running);
        assert_eq!(fixture.plan()?.intent().load()?, None);
        assert!(running(&process)?, "{phase:?}");
        control.close()?;
    }
    Ok(())
}

#[test]
fn repeated_interruptions_never_mint_another_boot_or_lose_the_process() -> TestResult {
    // Given: a restart that crashed after handover, and a successor that crashed too.
    let fixture = Fixture::new()?;
    let (control, _) = fixture.open()?;
    let record = spawn(&control, fixture.launch("printf ARMED; read gate")?)?;
    marker(&control, (record.id(), b"ARMED"))?;
    let process = record.lifecycle.process().ok_or("process")?.clone();
    let intent = crash_at(control, RestartPhase::HandedOver)?;
    let plan = fixture.plan()?;
    plan.boot(successor(&intent, fixture.identity)?)?.close()?;

    // When: a third control opens.
    let (control, resumed) = fixture.open()?;

    // Then: the boot is still the one the intent named, and the process is unchanged.
    let receipt = resumed.ok_or("the pending intent was not finished")?;
    assert_eq!(control.identity()?.server_boot_id(), intent.to);
    assert_eq!(receipt.preserved.len(), 1);
    assert!(running(&process)?);
    control.close()?;
    Ok(())
}

#[test]
fn an_intent_from_another_host_boot_or_lifetime_is_refused_and_kept() -> TestResult {
    // Given: a restart intent left behind by a crashed control.
    let fixture = Fixture::new()?;
    let (control, _) = fixture.open()?;
    let intent = crash_at(control, RestartPhase::IntentPersisted)?;

    // When: control opens after a host reboot, or as another runtime lifetime.
    let host = HostIdentity::new(fixture.identity.machine_id(), HostBootId::try_from(2)?);
    let rebooted = RuntimeIdentity::new(
        host,
        fixture.identity.runtime_id(),
        ServerBootId::try_from(1)?,
    );
    let rebooted = Control::open(fixture.plan()?, rebooted).map(drop);
    let foreign = RuntimeIdentity::new(
        HostIdentity::new(
            fixture.identity.machine_id(),
            fixture.identity.host_boot_id(),
        ),
        RuntimeId::try_from(8)?,
        ServerBootId::try_from(1)?,
    );
    let foreign = Control::open(fixture.plan()?, foreign).map(drop);

    // Then: neither claims a surviving process, and the intent is kept for inspection.
    assert_eq!(rebooted.map_err(|e| e.kind()), Err("host_rebooted"));
    assert_eq!(foreign.map_err(|e| e.kind()), Err("lifetime_mismatch"));
    assert_eq!(fixture.plan()?.intent().load()?, Some(intent));
    Ok(())
}

#[test]
fn a_failure_before_handover_keeps_the_old_boot_serving_and_signals_nothing() -> TestResult {
    // Given: a control whose next boot identity cannot be minted, with a live child.
    let mut fixture = Fixture::new()?;
    fixture.identity = RuntimeIdentity::new(
        HostIdentity::new(
            fixture.identity.machine_id(),
            fixture.identity.host_boot_id(),
        ),
        fixture.identity.runtime_id(),
        ServerBootId::try_from(u64::MAX)?,
    );
    let (control, _) = fixture.open()?;
    let record = spawn(&control, fixture.launch("printf ARMED; read gate")?)?;
    let process = record.lifecycle.process().ok_or("process")?.clone();

    // When: a restart is requested.
    let refused = control.restart(op()).map(drop).map_err(|e| e.kind());

    // Then: it fails typed, control still serves the old boot and the child still runs.
    assert_eq!(refused, Err("counter_overflow"));
    assert_eq!(control.phase()?, RestartPhase::Running);
    assert_eq!(
        control.identity()?.server_boot_id(),
        fixture.identity.server_boot_id()
    );
    assert_eq!(fixture.plan()?.intent().load()?, None);
    assert_eq!(
        control
            .with_terminals(|terminals, session| terminals.list(session))?
            .len(),
        1
    );
    assert!(running(&process)?);
    control.close()?;
    Ok(())
}

#[test]
fn a_launch_interrupted_before_spawn_evidence_stays_outcome_unknown_and_is_adopted() -> TestResult {
    // Given: a launch admitted whose spawn evidence this control has not consumed.
    let fixture = Fixture::new()?;
    let (control, _) = fixture.open()?;
    let launch_op = op();
    let launch = fixture.launch("printf ARMED; read gate")?;
    let admitted = control.with_terminals(|terminals, session| {
        terminals.run(Call::new(session, launch_op.clone()), launch)
    })?;
    assert_eq!(admitted.spawn, LaunchState::Pending);

    // When: control restarts.
    let receipt = control.restart(op())?;

    // Then: the launch is reported outcome_unknown with unproven delivery, never success,
    // while the successor adopted the one process the holder actually started.
    assert_eq!(
        receipt.interrupted_ops,
        vec![InterruptedOp::new(
            launch_op.clone(),
            InterruptedOutcome::OutcomeUnknown {
                delivery: Delivery::Unproven {}
            }
        )]
    );
    assert_eq!(
        receipt.transition().interrupted_ops,
        vec![launch_op.clone()]
    );
    let (adopted, state) = control.with_terminals(|terminals, session| {
        let adopted = terminals.await_spawn(admitted.id(), WAIT)?;
        Ok((adopted, terminals.operation(session, &launch_op)?.state))
    })?;
    assert_eq!(state, OpenEnum::Known(OpState::OutcomeUnknown));
    assert!(running(
        adopted.lifecycle.process().ok_or("adopted process")?
    )?);
    control.close()?;
    Ok(())
}

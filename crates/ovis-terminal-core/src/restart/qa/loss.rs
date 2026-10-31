use std::time::Instant;

use ovis_terminal_holder::protocol::{
    CoreBuildId, Exit, HOLDER_PROTOCOL_GENERATION, Hello, Message, PreviousCoreState, Signal,
    SignalKind, TerminalTarget,
};
use ovis_terminal_protocol::domain::restart::RestartPhase;

use super::barriers::crash_at;
use super::*;
use crate::restart::{IntentFile, LossEvidence, Lost, RestartIntent};

/// Adopts the holder as a probe core and waits for its exit report of `target`: the
/// holder reaped it. An exit already retained arrives in the welcome; nothing polls.
fn reaped(holder: &Holder, target: &TerminalTarget) -> TestResult<Exit> {
    let probe = holder.adopt(&Hello {
        generation: HOLDER_PROTOCOL_GENERATION,
        core_build: CoreBuildId::new("task-48-probe".into()),
        previous: PreviousCoreState::Crash,
    })?;
    let deadline = Instant::now().checked_add(WAIT).ok_or("deadline")?;
    let exit = loop {
        match probe.recv_timeout(deadline.saturating_duration_since(Instant::now()))? {
            Message::Welcome(welcome) => {
                let held = welcome
                    .terminals
                    .into_iter()
                    .find(|held| held.terminal_id == target.terminal_id);
                if let Some(exit) = held.and_then(|held| held.exit) {
                    break exit;
                }
            }
            Message::Exit(exit) if exit.target.terminal_id == target.terminal_id => break exit,
            _ => {}
        }
    };
    probe.release();
    Ok(exit)
}

fn first_held(intent: &RestartIntent) -> TestResult<Preserved> {
    Ok(intent
        .preserved
        .first()
        .ok_or("no preserved incarnation")?
        .clone())
}

use crate::restart::Preserved;

#[test]
fn a_child_killed_during_the_restart_is_lost_with_its_exit_and_never_preserved() -> TestResult {
    // Given: a live child and a restart that died right after handing the holder over.
    let fixture = Fixture::new()?;
    let (control, _) = fixture.open()?;
    let record = spawn(&control, fixture.launch("printf ARMED; read gate")?)?;
    marker(&control, (record.id(), b"ARMED"))?;
    let process = record.lifecycle.process().ok_or("process")?.clone();
    let target = record.holder_target.ok_or("holder target")?;
    let intent = crash_at(&fixture, control, RestartPhase::HandedOver)?;

    // When: our own child is killed before any successor adopts the holder.
    assert!(process.pid().get() > 1, "only a spawned child is signalled");
    fixture.holder.signal(&Signal {
        target: target.clone(),
        signal: SignalKind::Kill,
    })?;
    let exit = reaped(&fixture.holder, &target)?;
    let (control, boot) = fixture.open()?;

    // Then: the planned restart reports it lost with the holder's exit, never preserved.
    let receipt = planned(boot)?;
    assert_eq!(receipt.preserved, Vec::new());
    let evidence = LossEvidence::Exited {
        code: exit.code,
        signal: exit.signal,
    };
    assert_eq!(
        receipt.lost,
        vec![Lost::new(first_held(&intent)?, evidence)]
    );
    assert!(exit.signal.is_some() && exit.code.is_none(), "{exit:?}");
    let generation = process.generation().get();
    let ended: Vec<u64> = receipt
        .transition()
        .ended_incarnations
        .iter()
        .map(|i| i.get())
        .collect();
    assert_eq!(ended, vec![generation]);
    assert!(!running(&process)?);
    control.close()?;
    Ok(())
}

#[test]
fn a_recorded_identity_that_does_not_match_the_held_process_is_not_adopted_and_is_typed()
-> TestResult {
    // Given: a restart that died after handover, whose intent names another start token.
    let fixture = Fixture::new()?;
    let (control, _) = fixture.open()?;
    let record = spawn(&control, fixture.launch("printf ARMED; read gate")?)?;
    marker(&control, (record.id(), b"ARMED"))?;
    let process = record.lifecycle.process().ok_or("process")?.clone();
    let intent = crash_at(&fixture, control, RestartPhase::HandedOver)?;
    let mut tampered = serde_json::to_value(&intent)?;
    let token = tampered
        .pointer_mut("/preserved/0/process/process/start_token")
        .ok_or("the intent records no start token")?;
    *token = serde_json::json!("forged-start-token");
    let tampered: RestartIntent = serde_json::from_value(tampered)?;
    IntentFile::new(&fixture.root).store(&tampered)?;

    // When: the successor opens.
    let (control, boot) = fixture.open()?;

    // Then: the recorded identity is not claimed as preserved; the evidence names the
    // process the holder actually holds, which keeps running untouched.
    let receipt = planned(boot)?;
    assert_eq!(receipt.preserved, Vec::new());
    let lost = receipt.lost.first().ok_or("nothing reported lost")?;
    assert_eq!(lost.held, first_held(&tampered)?);
    assert_eq!(lost.evidence.kind(), "replaced");
    assert_eq!(
        lost.evidence,
        LossEvidence::Replaced {
            found: process.clone()
        }
    );
    assert!(running(&process)?);
    control.close()?;
    Ok(())
}

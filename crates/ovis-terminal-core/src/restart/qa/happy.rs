use ovis_terminal_protocol::domain::restart::{BoundaryPhase, InterruptedWait, RestartPhase};
use ovis_terminal_protocol::schema::ids::TerminalId as WireTerminalId;
use ovis_terminal_protocol::schema::scalar::ByteOffset;
use ovis_terminal_vt::{Geometry, HistoryLimit, VtConfig};

use super::*;
use crate::restart::RestartError;
use crate::screen::{Screen, ScreenBounds, ScreenConfig};
use crate::waits::{OutputOutcome, OutputWait, Waits};

fn hub(record: &Record) -> TestResult<Waits> {
    let target = record.holder_target.clone().ok_or("holder target")?;
    let config = ScreenConfig::new(
        VtConfig::new(Geometry::new(80, 24)?, HistoryLimit::new(100)),
        ScreenBounds::DEFAULT,
    );
    Ok(Waits::new(Screen::new(target, config)?))
}

#[test]
fn a_live_child_keeps_pid_token_and_lifetime_while_boot_changes_and_a_wait_ends_cancelled()
-> TestResult {
    // Given: a live child, a wait armed on its terminal and a boundary listener.
    let fixture = Fixture::new()?;
    let (control, resumed) = fixture.open()?;
    assert!(resumed.is_none());
    let record = spawn(&control, fixture.launch("printf ARMED; read gate")?)?;
    marker(&control, (record.id(), b"ARMED"))?;
    let process = record.lifecycle.process().ok_or("process")?.clone();
    let waits = hub(&record)?;
    let cursor = waits.cursor()?;
    let armed = control.waits().arm(InterruptedWait::new(
        op(),
        WireTerminalId::new(record.id().to_string()),
        ByteOffset::new(cursor.get()),
    ));
    let ticket = waits.output(OutputWait::new(cursor, b"NEVER-PRINTED")?, armed.scope())?;
    let boundaries = control.boundaries(2)?;
    let before = control.identity()?;

    // When: control restarts.
    let receipt = control.restart(op())?;

    // Then: the same process runs under the same lifetime, a new boot and a higher seq.
    let after = control.identity()?;
    assert_eq!(after.runtime_id(), before.runtime_id());
    assert_eq!(
        after.server_boot_id().get(),
        before.server_boot_id().get() + 1
    );
    assert_eq!(
        (receipt.from, receipt.to),
        (before.server_boot_id(), after.server_boot_id())
    );
    let preserved: Vec<_> = receipt.preserved.iter().map(|held| &held.process).collect();
    assert_eq!(preserved, vec![&process]);
    assert_eq!(receipt.lost, Vec::new());
    assert!(receipt.seq_after > receipt.seq_before, "{receipt:?}");
    assert!(running(&process)?);
    let adopted =
        control.with_terminals(|terminals, session| terminals.get(session, record.id()))?;
    assert_eq!(adopted.lifecycle.process(), Some(&process));
    assert_eq!(
        adopted.lifecycle.identity().server_boot_id(),
        after.server_boot_id()
    );
    assert_eq!(control.phase()?, RestartPhase::Running);
    assert_eq!(fixture.plan()?.intent().load()?, None);

    // And: the in-flight wait ended `cancelled` at its resume cursor, never as a match.
    let outcome = ticket.wait(Instant::now().checked_add(WAIT).ok_or("deadline")?)?;
    assert!(
        matches!(outcome, OutputOutcome::Cancelled { .. }),
        "{outcome:?}"
    );
    let cursors: Vec<_> = receipt
        .interrupted_waits
        .iter()
        .map(|wait| wait.resume_cursor.get())
        .collect();
    assert_eq!(cursors, vec![cursor.get()]);
    drop(armed);

    // And: the boundary was announced on both sides, completed with the preserved incarnation.
    let started = boundaries.recv_timeout(WAIT)?;
    let completed = boundaries.recv_timeout(WAIT)?;
    assert_eq!(
        (started.phase, completed.phase),
        (BoundaryPhase::Started, BoundaryPhase::Completed)
    );
    assert_eq!(
        completed.server_boot_id_new.as_str(),
        after.server_boot_id().to_string()
    );
    let evidence = completed.evidence.ok_or("recovery evidence")?;
    assert_eq!(
        evidence.restored,
        receipt.transition().preserved_incarnations
    );
    let wire = serde_json::to_value(receipt.transition())?;
    assert_eq!(
        wire.get("server_boot_id_old"),
        Some(&serde_json::json!(before.server_boot_id().to_string()))
    );
    assert_eq!(
        wire.get("preserved_incarnations"),
        Some(&serde_json::json!([process.generation().get().to_string()]))
    );
    control.close()?;
    Ok(())
}

#[test]
fn work_offered_inside_the_boundary_is_refused_as_restarting_not_queued_or_lost() -> TestResult {
    // Given: a restart on another thread, held at its start boundary by a rendezvous listener.
    let fixture = Fixture::new()?;
    let (control, _) = fixture.open()?;
    let control = std::sync::Arc::new(control);
    let boundaries = control.boundaries(1)?;
    let gate = control.boundaries(0)?;
    let restarting = std::sync::Arc::clone(&control);
    let restart = std::thread::spawn(move || restarting.restart(op()));

    // When: the boundary started, a read and a second restart are offered before the gate opens.
    assert_eq!(boundaries.recv_timeout(WAIT)?.phase, BoundaryPhase::Started);
    let read = control.with_terminals(|terminals, session| terminals.list(session));
    let again = control.restart(op());
    let phase = control.phase()?;
    assert_eq!(gate.recv_timeout(WAIT)?.phase, BoundaryPhase::Started);
    assert_eq!(gate.recv_timeout(WAIT)?.phase, BoundaryPhase::Completed);

    // Then: both are refused with the typed `restarting` kind, and the restart completes.
    assert_eq!(phase, RestartPhase::Suspended);
    assert!(
        matches!(
            read,
            Err(RestartError::Restarting {
                phase: RestartPhase::Suspended
            })
        ),
        "{read:?}"
    );
    assert_eq!(read.map(drop).map_err(|e| e.kind()), Err("restarting"));
    assert_eq!(again.map(drop).map_err(|e| e.kind()), Err("restarting"));
    let receipt = restart.join().map_err(|_| "restart thread")??;
    assert_eq!(
        boundaries.recv_timeout(WAIT)?.phase,
        BoundaryPhase::Completed
    );
    let listed = control.with_terminals(|terminals, session| terminals.list(session))?;
    assert_eq!(listed, Vec::new());
    assert_eq!(receipt.preserved, Vec::new());
    std::sync::Arc::try_unwrap(control)
        .map_err(|_| "shared control")?
        .close()?;
    Ok(())
}

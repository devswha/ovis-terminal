//! A caller that passed the readiness check just before a planned restart took the control
//! must be told `restarting`, never `control_stopped`: a runtime treats the latter as the
//! end of its holder link and would stop applying holder output for good. The reader holds
//! the lifecycle while it waits for holder messages, as the assembled runtime's actor does.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use super::*;
use std::sync::mpsc::RecvTimeoutError;

use crate::restart::RestartError;
use crate::terminals::TerminalError;

const RESTARTS: usize = 20;
/// How long the reader holds the lifecycle per entry, as the runtime's actor does.
const RECEIVE: Duration = Duration::from_millis(2);

#[test]
fn a_caller_racing_a_planned_restart_is_told_restarting_never_control_stopped() -> TestResult {
    // Given: a live child and a reader that keeps entering the lifecycle.
    let fixture = Fixture::new()?;
    let (control, _boot) = fixture.open()?;
    let record = spawn(&control, fixture.launch("printf ARMED; read gate")?)?;
    marker(&control, (record.id(), b"ARMED"))?;
    let done = AtomicBool::new(false);
    let (refusals, refused) = mpsc::channel();

    // When: control restarts repeatedly while the reader runs.
    std::thread::scope(|scope| -> TestResult {
        let reader = scope.spawn(|| {
            let mut entered = 0_usize;
            while !done.load(Ordering::Acquire) {
                let received = control.with_terminals(|terminals, _| {
                    match terminals.receive_message(RECEIVE) {
                        Ok(_) | Err(TerminalError::Notification(RecvTimeoutError::Timeout)) => {
                            Ok(())
                        }
                        Err(error) => Err(error),
                    }
                });
                match received {
                    Ok(()) => entered += 1,
                    Err(error) => {
                        if refusals.send(error.kind()).is_err() {
                            return entered;
                        }
                    }
                }
            }
            entered
        });
        let restarted = (0..RESTARTS).try_for_each(|_| control.restart(op()).map(drop));
        done.store(true, Ordering::Release);
        let entered = reader.join().map_err(|_| "reader panicked")?;
        restarted?;
        assert!(entered > 0, "the reader never entered the lifecycle");
        Ok(())
    })?;

    // Then: every refusal the reader saw was `restarting`.
    let kinds: std::collections::BTreeSet<&str> = refused.try_iter().collect();
    let restarting = RestartError::Restarting {
        phase: ovis_terminal_protocol::domain::restart::RestartPhase::Suspended,
    }
    .kind();
    assert!(
        kinds.iter().all(|kind| *kind == restarting),
        "refusals during planned restarts: {kinds:?}"
    );
    assert!(running(record.lifecycle.process().ok_or("process")?)?);
    Ok(())
}

//! Stop observation from declared status and exit evidence only.

use std::time::Instant;

use ovis_terminal_core::waits::{
    CancelScope, Observed, ProcessObservation, StopEvidence, StopWatch, WatchEnd,
};
use ovis_terminal_protocol::schema::methods::input::TurnObservation;
use ovis_terminal_protocol::schema::records::AgentState;
use ovis_terminal_protocol::schema::scalar::ReportSeq;

use super::TestResult;
use super::fixture::{bound, exit, fact, hub, print};

/// A working program that reports idle after the interrupt is stopped by declared status.
///
/// # Errors
///
/// Fails when the hub refuses a step.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn declared_stop_after_arming_is_stop_evidence() -> TestResult {
    let (waits, _screen) = hub(4096)?;
    waits.report(fact(1, AgentState::Working, (1, 0)))?;
    let ticket = waits.stop(StopWatch::new(), &CancelScope::new())?;
    print(&waits, b"Interrupted by user\r\n")?;
    waits.report(fact(2, AgentState::Idle, (1, 0)))?;
    let observed = ticket.wait(bound()?)?;
    assert_eq!(
        observed.end,
        WatchEnd::Stopped {
            evidence: StopEvidence::Status {
                seq: ReportSeq::new(2),
                state: AgentState::Idle
            }
        }
    );
    assert_eq!(observed.turn, TurnObservation::Ended);
    assert!(observed.stopped());
    Ok(())
}

/// Screen text and an idle report predating the interrupt are not stop evidence.
///
/// # Errors
///
/// Fails when the hub refuses a step.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn text_and_earlier_status_never_claim_a_stop() -> TestResult {
    let (waits, _screen) = hub(4096)?;
    waits.report(fact(1, AgentState::Idle, (0, 0)))?;
    let ticket = waits.stop(StopWatch::new(), &CancelScope::new())?;
    print(&waits, b"stopped\r\n^C\r\n")?;
    waits.report(fact(2, AgentState::Idle, (0, 0)))?;
    let observed: Observed = ticket.wait(Instant::now())?;
    assert_eq!(observed.end, WatchEnd::Timeout);
    assert_eq!(observed.process, ProcessObservation::Running);
    assert!(!observed.stopped());
    Ok(())
}

/// Exit of the incarnation is stop evidence; cancellation is not.
///
/// # Errors
///
/// Fails when the hub refuses a step.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn exit_stops_and_cancel_does_not() -> TestResult {
    let (waits, _screen) = hub(4096)?;
    let scope = CancelScope::new();
    let cancelled = waits.stop(StopWatch::new(), &scope)?;
    let exited = waits.stop(StopWatch::new(), &CancelScope::new())?;
    scope.cancel();
    let observed = cancelled.wait(bound()?)?;
    assert_eq!(observed.end, WatchEnd::Cancelled);
    assert!(!observed.stopped());
    exit(&waits, 0)?;
    let observed = exited.wait(bound()?)?;
    assert!(matches!(
        observed.end,
        WatchEnd::Stopped { evidence: StopEvidence::Exit { exit } } if exit.code == Some(0)
    ));
    assert_eq!(observed.process, ProcessObservation::Exited);
    Ok(())
}

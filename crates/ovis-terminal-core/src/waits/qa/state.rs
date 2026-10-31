//! State waits: evaluate first, then subscribe; typed ends; stale reports rejected.

use std::time::Instant;

use ovis_terminal_core::waits::{CancelScope, ProcessState, Satisfied, StateOutcome, StateWait};
use ovis_terminal_protocol::schema::records::AgentState;
use ovis_terminal_protocol::schema::scalar::ReportSeq;

use super::TestResult;
use super::fixture::{bound, exit, fact, hub};

/// An already idle program answers an idle wait at once, without arming anything.
///
/// # Errors
///
/// Fails when the hub refuses a step.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn already_satisfied_state_returns_immediately() -> TestResult {
    let (waits, _screen) = hub(4096)?;
    waits.report(fact(1, AgentState::Idle, (0, 0)))?;
    let ticket = waits.state(
        StateWait::new(&[AgentState::Idle], false)?,
        &CancelScope::new(),
    )?;
    assert_eq!(
        ticket.wait(Instant::now())?,
        StateOutcome::AlreadySatisfied {
            satisfied: Satisfied::Status {
                seq: ReportSeq::new(1),
                state: AgentState::Idle
            }
        }
    );
    exit(&waits, 0)?;
    let exited = waits.state(StateWait::new(&[], true)?, &CancelScope::new())?;
    assert!(matches!(
        exited.wait(Instant::now())?,
        StateOutcome::AlreadySatisfied { satisfied: Satisfied::Exit { exit } } if exit.code == Some(0)
    ));
    Ok(())
}

/// A wait armed before the report is matched by it; other ends are typed, never a match.
///
/// # Errors
///
/// Fails when the hub refuses a step.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn armed_state_wait_ends_by_report_exit_loss_or_timeout() -> TestResult {
    let (waits, _screen) = hub(4096)?;
    let idle = waits.state(
        StateWait::new(&[AgentState::Idle], false)?,
        &CancelScope::new(),
    )?;
    let blocked = waits.state(
        StateWait::new(&[AgentState::Blocked], false)?,
        &CancelScope::new(),
    )?;
    let timed = waits.state(
        StateWait::new(&[AgentState::Done], false)?,
        &CancelScope::new(),
    )?;
    waits.report(fact(1, AgentState::Working, (1, 0)))?;
    waits.report(fact(2, AgentState::Idle, (1, 0)))?;
    assert!(matches!(
        idle.wait(bound()?)?,
        StateOutcome::Matched {
            satisfied: Satisfied::Status {
                state: AgentState::Idle,
                ..
            }
        }
    ));
    assert_eq!(timed.wait(Instant::now())?, StateOutcome::Timeout);
    waits.lost()?;
    assert_eq!(blocked.wait(bound()?)?, StateOutcome::Lost);
    assert_eq!(waits.process()?, ProcessState::Lost);
    Ok(())
}

/// Stale and post-exit reports are rejected with typed kinds and change nothing.
///
/// # Errors
///
/// Fails when the hub refuses a valid step.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn stale_and_late_reports_are_rejected() -> TestResult {
    let (waits, _screen) = hub(4096)?;
    waits.report(fact(5, AgentState::Working, (1, 0)))?;
    let Err(stale) = waits.report(fact(5, AgentState::Idle, (1, 0))) else {
        return Err("a report repeating the sequence was applied".into());
    };
    assert_eq!(stale.kind(), "stale_seq");
    let still = waits.state(
        StateWait::new(&[AgentState::Working], false)?,
        &CancelScope::new(),
    )?;
    assert!(matches!(
        still.wait(Instant::now())?,
        StateOutcome::AlreadySatisfied { .. }
    ));
    exit(&waits, 1)?;
    let Err(late) = waits.report(fact(6, AgentState::Idle, (1, 0))) else {
        return Err("a report after exit was applied".into());
    };
    assert_eq!(late.kind(), "finished");
    let Err(empty) = StateWait::new(&[], false) else {
        return Err("an empty condition was accepted".into());
    };
    assert_eq!(empty.kind(), "invalid_params");
    Ok(())
}

/// Cancelling a scope ends every wait armed under it and detaches the observers.
///
/// # Errors
///
/// Fails when the hub refuses a step.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn cancel_scope_ends_and_detaches_its_waits() -> TestResult {
    let (waits, _screen) = hub(4096)?;
    let scope = CancelScope::new();
    let first = waits.state(StateWait::new(&[AgentState::Idle], false)?, &scope)?;
    let second = waits.state(StateWait::new(&[], true)?, &scope)?;
    let other = waits.state(
        StateWait::new(&[AgentState::Idle], false)?,
        &CancelScope::new(),
    )?;
    scope.cancel();
    waits.report(fact(1, AgentState::Idle, (0, 0)))?;
    assert_eq!(first.wait(bound()?)?, StateOutcome::Cancelled);
    assert_eq!(second.wait(bound()?)?, StateOutcome::Cancelled);
    assert!(matches!(
        other.wait(bound()?)?,
        StateOutcome::Matched { .. }
    ));
    assert!(scope.is_cancelled());
    Ok(())
}

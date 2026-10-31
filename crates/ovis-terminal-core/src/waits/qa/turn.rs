//! Turn waits bound to one send or one turn number.

use std::thread;
use std::time::Instant;

use ovis_terminal_core::input::receipts::InputSeq;
use ovis_terminal_core::waits::{CancelScope, TurnBinding, TurnOutcome};
use ovis_terminal_protocol::schema::records::AgentState;
use ovis_terminal_protocol::schema::scalar::{ReportSeq, TurnNumber};

use super::TestResult;
use super::fixture::{bound, exit, fact, hub};

/// A turn already running when the send arrived never satisfies the send's wait.
///
/// # Errors
///
/// Fails when the hub refuses a step or the waiting thread panics.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn only_the_turn_after_the_send_completes_its_wait() -> TestResult {
    let (waits, _screen) = hub(4096)?;
    waits.report(fact(1, AgentState::Working, (1, 0)))?;
    let ticket = waits.turn(TurnBinding::Send(InputSeq::new(1)), &CancelScope::new())?;
    let earlier = waits.turn(TurnBinding::Turn(TurnNumber::new(1)), &CancelScope::new())?;
    let deadline = bound()?;
    let waiter = thread::spawn(move || ticket.wait(deadline));
    waits.report(fact(2, AgentState::Idle, (1, 0)))?;
    assert_eq!(
        earlier.wait(bound()?)?,
        TurnOutcome::Completed {
            turn: TurnNumber::new(1),
            ended_seq: ReportSeq::new(2)
        }
    );
    waits.report(fact(3, AgentState::Working, (2, 1)))?;
    waits.report(fact(4, AgentState::Blocked, (2, 1)))?;
    let outcome = waiter.join().map_err(|_| "waiter panicked")??;
    assert_eq!(
        outcome,
        TurnOutcome::Blocked {
            turn: TurnNumber::new(2),
            ended_seq: ReportSeq::new(4)
        },
        "turn 1 ended first, yet only turn 2 started after the send"
    );
    let replay = waits.turn(TurnBinding::Send(InputSeq::new(1)), &CancelScope::new())?;
    assert_eq!(replay.wait(Instant::now())?, outcome);
    Ok(())
}

/// The bound passing is `timeout`, never completion; exit and cancel are their own ends.
///
/// # Errors
///
/// Fails when the hub refuses a step.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn timeout_exit_and_cancel_never_complete_a_turn() -> TestResult {
    let (waits, _screen) = hub(4096)?;
    let none = waits.turn(TurnBinding::Send(InputSeq::new(1)), &CancelScope::new())?;
    assert_eq!(
        none.wait(Instant::now())?,
        TurnOutcome::Timeout { turn: None }
    );
    waits.report(fact(1, AgentState::Working, (1, 1)))?;
    let running = waits.turn(TurnBinding::Send(InputSeq::new(1)), &CancelScope::new())?;
    assert_eq!(
        running.wait(Instant::now())?,
        TurnOutcome::Timeout {
            turn: Some(TurnNumber::new(1))
        }
    );
    let scope = CancelScope::new();
    let cancelled = waits.turn(TurnBinding::Turn(TurnNumber::new(1)), &scope)?;
    scope.cancel();
    assert_eq!(
        cancelled.wait(bound()?)?,
        TurnOutcome::Cancelled {
            turn: Some(TurnNumber::new(1))
        }
    );
    let exited = waits.turn(TurnBinding::Send(InputSeq::new(1)), &CancelScope::new())?;
    exit(&waits, 130)?;
    assert!(matches!(
        exited.wait(bound()?)?,
        TurnOutcome::Exited { turn: Some(turn), exit } if turn == TurnNumber::new(1) && exit.code == Some(130)
    ));
    Ok(())
}

/// A turn number that passed unretained is `unknown_turn`; a future one waits.
///
/// # Errors
///
/// Fails when the hub refuses a valid step.
///
/// # Panics
///
/// When an asserted outcome does not hold.
pub fn unknown_and_stale_turns_are_rejected() -> TestResult {
    let (waits, _screen) = hub(4096)?;
    waits.report(fact(1, AgentState::Working, (3, 0)))?;
    let Err(unknown) = waits.turn(TurnBinding::Turn(TurnNumber::new(2)), &CancelScope::new())
    else {
        return Err("a turn that never started was armed".into());
    };
    assert_eq!(unknown.kind(), "unknown_turn");
    let future = waits.turn(TurnBinding::Turn(TurnNumber::new(4)), &CancelScope::new())?;
    waits.report(fact(2, AgentState::Idle, (3, 0)))?;
    let Err(stale) = waits.report(fact(3, AgentState::Working, (3, 0))) else {
        return Err("a turn number was reused".into());
    };
    assert_eq!(stale.kind(), "stale_seq");
    assert_eq!(
        future.wait(Instant::now())?,
        TurnOutcome::Timeout { turn: None }
    );
    let json = serde_json::to_string(&TurnOutcome::Completed {
        turn: TurnNumber::new(u64::MAX),
        ended_seq: ReportSeq::new(u64::MAX),
    })?;
    assert_eq!(
        json,
        r#"{"outcome":"completed","turn":"18446744073709551615","ended_seq":"18446744073709551615"}"#
    );
    assert!(serde_json::from_str::<TurnOutcome>(&json).is_ok());
    Ok(())
}

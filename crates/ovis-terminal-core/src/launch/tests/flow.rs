//! Readiness gating, single release and settlement of a queued first message.

use std::cell::Cell;
use std::time::{Duration, Instant};

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_protocol::domain::template_launch::{FirstMessageReason, FirstMessageState};
use ovis_terminal_protocol::schema::records::AgentState;
use ovis_terminal_protocol::schema::scalar::{EventSeq, OpenEnum, ReportSeq, TurnNumber};

use super::fixture::{TestResult, bound, hub, queued};
use crate::input::receipts::InputSeq;
use crate::launch::{FirstMessages, Settlement, Verdict, arm, await_ready};
use crate::waits::{CancelScope, Satisfied, StateOutcome, StatusFact, TurnPosition};

fn fact(seq: u64, state: AgentState) -> StatusFact {
    StatusFact::new(
        ReportSeq::new(seq),
        state,
        TurnPosition::new(TurnNumber::new(seq), InputSeq::new(0)),
    )
}

#[test]
fn a_declaration_before_the_wait_is_armed_releases_exactly_once() -> TestResult {
    let waits = hub()?;
    waits.report(fact(1, AgentState::Idle))?;
    let ticket = arm(&waits, &[AgentState::Idle], &CancelScope::new())?;
    assert_eq!(await_ready(ticket, bound()?)?, Verdict::Ready);
    let book = FirstMessages::new();
    let message = queued("launcher", "run the build")?;
    let launch = message.record.launch_op.clone();
    book.accept(message)?;
    let writes = Cell::new(0_u32);
    let write = |text: &str| {
        assert_eq!(text, "run the build");
        writes.set(writes.get() + 1);
        Ok(EventSeq::new(1))
    };
    let sent = book.release(&launch, write)?;
    assert_eq!(sent.state, OpenEnum::Known(FirstMessageState::Sent));
    assert_eq!(sent.queued_seq, Some(EventSeq::new(1)));
    let again = book.release(&launch, |_| Ok(EventSeq::new(2)))?;
    assert_eq!((writes.get(), again), (1, sent));
    Ok(())
}

#[test]
fn a_working_declaration_does_not_release_and_the_later_idle_one_does() -> TestResult {
    let waits = hub()?;
    let ticket = arm(
        &waits,
        &[AgentState::Idle, AgentState::Done],
        &CancelScope::new(),
    )?;
    waits.report(fact(1, AgentState::Working))?;
    waits.report(fact(2, AgentState::Idle))?;
    let outcome = ticket.wait(bound()?)?;
    let StateOutcome::Matched {
        satisfied: Satisfied::Status { seq, state },
    } = outcome
    else {
        panic!("expected a match by the idle report, got {outcome:?}");
    };
    assert_eq!((seq, state), (ReportSeq::new(2), AgentState::Idle));
    Ok(())
}

#[test]
fn a_program_that_never_declares_readiness_expires_with_zero_writes() -> TestResult {
    let waits = hub()?;
    let ticket = arm(&waits, &[AgentState::Idle], &CancelScope::new())?;
    waits.report(fact(1, AgentState::Working))?;
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(50))
        .ok_or("clock overflow")?;
    let verdict = await_ready(ticket, deadline)?;
    assert_eq!(verdict, Verdict::Settle(Settlement::Expired));
    let book = FirstMessages::new();
    let message = queued("launcher", "never")?;
    let launch = message.record.launch_op.clone();
    book.accept(message)?;
    let expired = book.settle(&launch, Settlement::Expired)?;
    assert_eq!(expired.state, OpenEnum::Known(FirstMessageState::Expired));
    assert_eq!(
        expired.reason,
        Some(OpenEnum::Known(FirstMessageReason::DeadlinePassed))
    );
    let after = book.release(&launch, |_| panic!("an expired message is never written"))?;
    assert_eq!(after, expired);
    Ok(())
}

#[test]
fn a_lost_link_or_a_cancelled_scope_cancels_without_release() -> TestResult {
    let waits = hub()?;
    let ticket = arm(&waits, &[AgentState::Idle], &CancelScope::new())?;
    waits.lost()?;
    let lost = Verdict::Settle(Settlement::Cancelled(FirstMessageReason::LinkLost));
    assert_eq!(await_ready(ticket, bound()?)?, lost);
    let scope = CancelScope::new();
    let ticket = arm(&hub()?, &[AgentState::Idle], &scope)?;
    scope.cancel();
    let stopped = Verdict::Settle(Settlement::Cancelled(FirstMessageReason::RuntimeStopped));
    assert_eq!(await_ready(ticket, bound()?)?, stopped);
    Ok(())
}

#[test]
fn a_message_is_read_and_queued_only_by_its_requesting_principal() -> TestResult {
    let book = FirstMessages::new();
    let message = queued("launcher", "hello")?;
    let launch = message.record.launch_op.clone();
    let first = book.accept(message.clone())?;
    assert_eq!(book.accept(message.clone())?, first);
    let mut foreign = message;
    foreign.principal = PrincipalId::parse("intruder")?;
    assert_eq!(
        book.accept(foreign).map_err(|error| error.kind()),
        Err("forbidden")
    );
    let intruder = PrincipalId::parse("intruder")?;
    assert_eq!(
        book.get(&intruder, &launch).map_err(|error| error.kind()),
        Err("forbidden")
    );
    let unknown = queued("launcher", "x")?.record.launch_op;
    let launcher = PrincipalId::parse("launcher")?;
    assert_eq!(
        book.get(&launcher, &unknown).map_err(|error| error.kind()),
        Err("not_found")
    );
    assert_eq!(book.get(&launcher, &launch)?, first);
    Ok(())
}

#[test]
fn a_settled_wait_answers_the_final_state_and_an_elapsed_one_the_current() -> TestResult {
    let book = std::sync::Arc::new(FirstMessages::new());
    let message = queued("launcher", "hello")?;
    let launch = message.record.launch_op.clone();
    book.accept(message)?;
    let launcher = PrincipalId::parse("launcher")?;
    let current = book.wait_settled((&launcher, &launch), Instant::now())?;
    assert!(current.is_pending());
    let settler = {
        let (book, launch) = (std::sync::Arc::clone(&book), launch.clone());
        std::thread::spawn(move || {
            book.settle(
                &launch,
                Settlement::Cancelled(FirstMessageReason::ProcessExited),
            )
        })
    };
    let settled = book.wait_settled((&launcher, &launch), bound()?)?;
    let joined = settler.join().map_err(|_| "settler panicked")??;
    assert_eq!(settled, joined);
    assert_eq!(
        settled.reason,
        Some(OpenEnum::Known(FirstMessageReason::ProcessExited))
    );
    Ok(())
}

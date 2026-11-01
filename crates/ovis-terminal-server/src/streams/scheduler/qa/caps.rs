//! Session caps on control slots, waiting output and event retention: typed refusal before
//! anything is queued, and room returned when the holder lets go.

use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_vt::{Geometry, OutputOffset};
use tokio::sync::broadcast::error::TryRecvError;

use super::super::{
    ActorConfig, Budget, BudgetScope, Engine, OutputEvent, SchedulerError, SessionBudgets,
    TerminalSetup, spawn_terminal,
};
use super::{await_event, spawn, vt};

fn with_budget(budget: std::sync::Arc<Budget>) -> TerminalSetup {
    let limits = Limits::v1();
    let session = SessionBudgets::advertised(&limits).with(budget);
    TerminalSetup::new(ActorConfig::advertised(&limits), session)
}

#[tokio::test]
async fn an_exhausted_session_control_budget_refuses_but_never_refuses_stop() {
    let slots = Budget::new(BudgetScope::SessionControl, 1);
    let (handle, _feed, task) = spawn(&with_budget(std::sync::Arc::clone(&slots)));
    let elsewhere = slots
        .reserve(1)
        .expect("another terminal holds the only slot");

    let geometry = Geometry::new(100, 30).expect("valid geometry");
    let refused = handle.resize(geometry).await.expect_err("no session slot");
    assert!(matches!(
        refused,
        SchedulerError::Exhausted {
            scope: BudgetScope::SessionControl,
            held: 1,
            limit: 1,
            ..
        }
    ));
    assert_eq!(refused.kind(), "backpressure");
    drop(elsewhere);
    handle
        .resize(geometry)
        .await
        .expect("served once a slot is free");
    assert_eq!(slots.held(), 0, "the served request returned its slot");

    let _held_again = slots.reserve(1).expect("slot");
    handle.stop().expect("stop needs no session slot");
    let report = task.join().await.expect("joined");
    assert_eq!((report.controls, report.resized), (1, 1));
}

#[tokio::test]
async fn output_waits_for_the_session_output_budget_without_loss() {
    let output_budget = Budget::new(BudgetScope::SessionOutput, 16 * 1024);
    let (handle, feed, task) = spawn(&with_budget(std::sync::Arc::clone(&output_budget)));
    let mut output = handle.subscribe_output();
    let elsewhere = output_budget
        .reserve(16 * 1024)
        .expect("another terminal holds it");

    // Given a chunk that cannot fit the session budget, the actor stashes it: after a
    // control request is served, nothing was accepted (current-thread runtime).
    let chunk = vec![b'y'; 4096];
    feed.send(&chunk).await.expect("the feed takes it");
    handle
        .resize(Geometry::new(90, 24).expect("valid geometry"))
        .await
        .expect("served");
    assert!(
        matches!(output.try_recv(), Err(TryRecvError::Empty)),
        "nothing accepted"
    );
    assert_eq!(output_budget.held(), 16 * 1024);

    // When the other holder releases, the stashed chunk is accepted and parsed.
    drop(elsewhere);
    let end = OutputOffset::new(4096);
    await_event(&mut output, |event| match event {
        OutputEvent::Parsed { offset } if *offset == end => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(output_budget.held(), 0, "parsed bytes are returned");
    handle.stop().expect("stop queued");
    let report = task.join().await.expect("joined");
    assert_eq!((report.output_waits, report.parsed_bytes), (1, 4096));
}

#[tokio::test]
async fn event_retention_is_reserved_per_terminal_against_the_session() {
    let limits = Limits::v1();
    let config = ActorConfig::advertised(&limits);
    let per_terminal = config.retention_bytes();
    let events = Budget::new(BudgetScope::SessionEvents, per_terminal.saturating_mul(2));
    let setup = with_budget(std::sync::Arc::clone(&events));
    let (first, _, first_task) = spawn(&setup);
    let (_second, _, _second_task) = spawn(&setup);
    assert_eq!(events.held(), per_terminal.saturating_mul(2));

    let refused = spawn_terminal(Engine::new(vt()), tokio::io::sink(), &setup)
        .expect_err("a third terminal's retention does not fit");
    assert!(matches!(
        refused,
        SchedulerError::Exhausted {
            scope: BudgetScope::SessionEvents,
            ..
        }
    ));
    assert_eq!(refused.kind(), "backpressure");

    first.stop().expect("stop queued");
    first_task.join().await.expect("joined");
    assert_eq!(
        events.held(),
        per_terminal,
        "a stopped terminal returns its retention"
    );
    let third = spawn_terminal(Engine::new(vt()), tokio::io::sink(), &setup);
    assert!(third.is_ok(), "room was returned");
}

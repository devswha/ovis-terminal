use std::sync::Arc;
use std::time::Instant;

use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_vt::Geometry;

use super::super::{
    ActorConfig, BudgetScope, ByteBudget, CancelOutcome, SchedulerError, TerminalEvent,
    TerminalSetup, spawn_terminal,
};
use super::{CONTROL_BOUND, WAIT, await_event, setup, vt};

/// One byte past the advertised 16 MiB terminal input queue.
const CAP_PLUS_ONE: u64 = 16 * 1024 * 1024 + 1;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overflow_is_refused_before_admission_and_cancel_stays_serviceable() {
    // Given a program that never reads stdin: its pipe holds 1 KiB and nobody drains it.
    let (sink, stdin_never_read) = tokio::io::duplex(1024);
    let (handle, _output, task) = spawn_terminal(vt(), sink, &setup());
    let mut events = handle.subscribe();

    // When one byte past the queue bound is offered, it is refused before any buffer.
    let refused = handle
        .reserve(CAP_PLUS_ONE)
        .expect_err("one past the bound");
    assert_eq!(refused.kind(), "input_overflow");
    assert!(matches!(
        refused,
        SchedulerError::Exhausted {
            scope: BudgetScope::TerminalInput,
            offered: CAP_PLUS_ONE,
            held: 0,
            ..
        }
    ));
    assert!(!refused.to_string().contains("input_overflow"));
    assert_eq!(handle.input_held(), 0, "a refusal holds nothing");

    // And entries larger than the pipe are admitted; the first one blocks the writer.
    let mut tickets = Vec::new();
    for _ in 0..3 {
        let held = handle.reserve(4096).expect("fits");
        let started = Instant::now();
        let receipt = handle
            .admit(held, vec![b'x'; 4096])
            .await
            .expect("admitted");
        assert!(started.elapsed() <= CONTROL_BOUND);
        tickets.push(receipt.ticket);
    }
    let [stuck, queued, last] = tickets[..] else {
        panic!("three tickets");
    };
    await_event(&mut events, |event| match event {
        TerminalEvent::Writing { ticket } if *ticket == stuck => Some(()),
        _ => None,
    })
    .await;

    // Then control is served at once while the writer is stuck on the full pipe.
    let started = Instant::now();
    assert_eq!(
        handle.cancel(queued).await.ok(),
        Some(CancelOutcome::Withdrawn)
    );
    assert_eq!(
        handle.cancel(stuck).await.ok(),
        Some(CancelOutcome::InFlight)
    );
    let geometry = Geometry::new(100, 30).expect("valid geometry");
    handle.resize(geometry).await.expect("resize served");
    assert!(
        started.elapsed() <= CONTROL_BOUND,
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        handle.input_held(),
        2 * 4096,
        "the withdrawn entry released its bytes"
    );

    handle.stop().expect("stop queued");
    let report = tokio::time::timeout(WAIT, task.join())
        .await
        .expect("the actor stops while its writer is stuck")
        .expect("joined");
    assert_eq!(report.admitted, 3);
    assert_eq!(report.withdrawn, 1);
    assert_eq!(report.unsent, 1, "ticket {last} was never started");
    assert_eq!(report.cut_off, Some(stuck));
    assert_eq!(
        report.writer, None,
        "the stuck writer was aborted, not awaited"
    );
    drop(stdin_never_read);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_session_bound_refuses_and_returns_the_terminal_reservation() {
    let limits = Limits::v1();
    let session = ByteBudget::new(BudgetScope::SessionInput, 8 * 1024);
    let setup = TerminalSetup::new(ActorConfig::advertised(&limits), Arc::clone(&session));
    let (first, _, first_task) = spawn_terminal(vt(), tokio::io::sink(), &setup);
    let (second, _, second_task) = spawn_terminal(vt(), tokio::io::sink(), &setup);

    let held = first.reserve(6 * 1024).expect("fits the session");
    let refused = second
        .reserve(4 * 1024)
        .expect_err("passes the session bound");
    assert!(matches!(
        refused,
        SchedulerError::Exhausted {
            scope: BudgetScope::SessionInput,
            held: 6144,
            limit: 8192,
            ..
        }
    ));
    assert_eq!(
        second.input_held(),
        0,
        "the terminal reservation was returned"
    );
    assert_eq!(session.held(), 6 * 1024);

    let over = first.admit(held, vec![0; 6 * 1024 + 1]).await;
    assert_eq!(
        over.map_err(|error| error.kind()).err(),
        Some("invalid_params")
    );
    assert_eq!(
        session.held(),
        0,
        "a refused admission releases its reservation"
    );

    for (handle, task) in [(first, first_task), (second, second_task)] {
        handle.stop().expect("stop queued");
        let report = task.join().await.expect("joined");
        assert_eq!(report.admitted, 0);
    }
}

#[test]
fn every_refusal_maps_to_a_typed_retryable_wire_kind() {
    let cases = [
        (
            SchedulerError::Exhausted {
                scope: BudgetScope::TerminalInput,
                offered: 2,
                held: 0,
                limit: 1,
            },
            "input_overflow",
        ),
        (
            SchedulerError::QueueFull {
                lane: "control",
                capacity: 16,
            },
            "backpressure",
        ),
        (
            SchedulerError::BlockingSaturated { limit: 4 },
            "backpressure",
        ),
        (SchedulerError::WriterBusy { capacity: 8 }, "backpressure"),
    ];
    for (error, kind) in cases {
        assert_eq!(error.kind(), kind);
        let wire = serde_json::to_value(error.boundary().data()).expect("encodes");
        assert_eq!(wire.get("kind"), Some(&serde_json::json!(kind)));
        assert_eq!(wire.get("retryable"), Some(&serde_json::json!(true)));
        assert!(!error.to_string().contains(kind));
    }
}

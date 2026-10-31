//! Control-first ordering, pinned by counts rather than clocks.
//!
//! These tests run on the current-thread runtime: the actor runs only while the test task
//! waits, so the parse turns observed before a control request is sent are exactly the
//! turns the actor had run. Control events carry the turns run before they committed.

use std::num::NonZeroUsize;

use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_vt::{Geometry, OutputOffset, ParseBudget};
use tokio::sync::broadcast;

use super::super::{ActorConfig, CancelOutcome, OutputEvent, TerminalEvent, TerminalSetup};
use super::{await_event, spawn};

const TURN: usize = 1024;
const TURNS: u64 = 32;

fn one_kib_turns() -> TerminalSetup {
    let limits = Limits::v1();
    let budget = ParseBudget::new(NonZeroUsize::new(TURN).expect("nonzero"));
    TerminalSetup::advertised(
        ActorConfig::advertised(&limits).with_parse_budget(budget),
        &limits,
    )
}

fn turns_at(offset: OutputOffset) -> u64 {
    offset
        .get()
        .checked_div(u64::try_from(TURN).expect("fits"))
        .unwrap_or(0)
}

/// Turns the actor has run, read from every `Parsed` event published so far.
fn turns_so_far(output: &mut broadcast::Receiver<OutputEvent>, mut seen: u64) -> u64 {
    while let Ok(event) = output.try_recv() {
        if let OutputEvent::Parsed { offset } = event {
            seen = turns_at(offset);
        }
    }
    seen
}

async fn first_turn(output: &mut broadcast::Receiver<OutputEvent>) -> u64 {
    let first = await_event(output, |event| match event {
        OutputEvent::Parsed { offset } => Some(*offset),
        OutputEvent::Chunk { .. } => None,
    })
    .await;
    let seen = turns_at(first);
    turns_so_far(output, seen)
}

#[tokio::test]
async fn a_resize_waits_behind_at_most_one_parse_turn() {
    let (handle, feed, task) = spawn(&one_kib_turns());
    let mut output = handle.subscribe_output();
    let mut events = handle.subscribe();
    // Given 32 KiB of output, which is 32 one-KiB parse turns, and the first turn run.
    let bytes = vec![b'x'; TURN.saturating_mul(32)];
    feed.send(&bytes).await.expect("the actor takes output");
    let before = first_turn(&mut output).await;
    assert!(
        before < TURNS,
        "parse work remains when the control request is sent"
    );

    // When a resize is sent while turns are pending.
    let geometry = Geometry::new(100, 30).expect("valid geometry");
    handle.resize(geometry).await.expect("resize served");

    // Then it commits after at most one more parse turn, not after the backlog.
    let after = await_event(&mut events, |event| match event {
        TerminalEvent::Resized { after_turns, .. } => Some(*after_turns),
        _ => None,
    })
    .await;
    assert!(
        after <= before.saturating_add(1),
        "resize committed after {after} turns; {before} had run when it was sent"
    );
    assert!(
        after < TURNS,
        "the resize did not wait for the whole backlog"
    );
    let end = OutputOffset::new(u64::try_from(bytes.len()).expect("fits"));
    await_event(&mut output, |event| match event {
        OutputEvent::Parsed { offset } if *offset == end => Some(()),
        _ => None,
    })
    .await;
    handle.stop().expect("stop queued");
    let report = task.join().await.expect("joined");
    assert_eq!(report.parse_turns, TURNS);
}

#[tokio::test]
async fn admissions_and_cancels_overtake_pending_parse_turns_while_stdin_is_stuck() {
    let (sink, stdin_never_read) = tokio::io::duplex(64);
    let setup = one_kib_turns();
    let (handle, feed, task) =
        super::super::spawn_terminal(super::super::Engine::new(super::vt()), sink, &setup)
            .expect("spawns");
    let mut output = handle.subscribe_output();
    let mut events = handle.subscribe();
    // Given a writer stuck on a program that never reads, and 32 turns of output.
    let first = handle.reserve(256).expect("fits");
    let stuck = handle
        .admit(first, vec![b'a'; 256])
        .await
        .expect("admitted");
    feed.send(&vec![b'x'; TURN.saturating_mul(32)])
        .await
        .expect("the actor takes output");
    let before = first_turn(&mut output).await;

    // When another entry is admitted and then withdrawn.
    let held = handle.reserve(8).expect("fits");
    let queued = handle
        .admit(held, b"echo hi\r".to_vec())
        .await
        .expect("admitted");
    let withdrawn = handle.cancel(queued.ticket).await.expect("cancel served");
    assert_eq!(withdrawn, CancelOutcome::Withdrawn);
    assert_eq!(
        handle.cancel(stuck.ticket).await.ok(),
        Some(CancelOutcome::InFlight)
    );

    // Then both committed within one parse turn of being sent.
    let mut committed = Vec::new();
    while committed.len() < 3 {
        let turns = await_event(&mut events, |event| match event {
            TerminalEvent::Admitted { after_turns, .. }
            | TerminalEvent::Withdrawn { after_turns, .. } => Some(*after_turns),
            _ => None,
        })
        .await;
        committed.push(turns);
    }
    let [_, admitted, cancelled] = committed[..] else {
        panic!("three commits");
    };
    assert!(
        admitted <= before.saturating_add(1),
        "admitted after {admitted} turns, sent after {before}"
    );
    assert!(
        cancelled <= admitted.saturating_add(1),
        "withdrawn after {cancelled} turns"
    );
    assert!(
        before < TURNS,
        "parse work remained when the requests were sent"
    );
    let end = OutputOffset::new(u64::try_from(TURN.saturating_mul(32)).expect("fits"));
    await_event(&mut output, |event| match event {
        OutputEvent::Parsed { offset } if *offset == end => Some(()),
        _ => None,
    })
    .await;
    handle.stop().expect("stop queued");
    let report = task.join().await.expect("joined");
    assert_eq!((report.withdrawn, report.cut_off), (1, Some(stuck.ticket)));
    assert_eq!(
        report.parse_turns, TURNS,
        "one budget per turn, never the whole backlog"
    );
    drop(stdin_never_read);
}

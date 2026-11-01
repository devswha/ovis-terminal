use ovis_terminal_core::subscriptions::{LossCause, Selector, Start};
use ovis_terminal_protocol::schema::scalar::EventSeq;

use super::fixture::{Bounds, Data, TestResult, World, terminal};
use super::pulls::{scan_to, until_loss_event, until_missed};

#[test]
fn a_withheld_consumer_recovers_from_its_durable_marker_and_continues() -> TestResult {
    // Given a two-event bound, a withheld consumer and a healthy witness.
    let mut bounds = Bounds::v1()?;
    bounds.queue.queue_events = 2;
    let world = World::open(bounds)?;
    let witness = world.subscribe(Start::Head, Selector::all())?;
    let withheld = world.subscribe(Start::Head, Selector::all())?;
    let mut first = None;
    let mut seen = Vec::new();
    for value in ["one", "two", "three"] {
        let seq = world.set(1, value)?.seq.get();
        first.get_or_insert(seq);
        seen.extend(scan_to(&witness.feed, seq)?.0);
    }
    let (marker_seq, marker) = until_loss_event(&witness.feed, &seen)?;
    // When the withheld consumer finally reads, and the next change commits.
    let (before, recovery) = until_missed(&withheld.feed)?;
    let next = world.set(2, "after")?.seq.get();
    let (after, _) = scan_to(&withheld.feed, next)?;
    // Then it gets the already durable marker plus a fresh snapshot instead of a partial
    // queue, and delivery resumes strictly after that snapshot.
    assert!(
        before.is_empty(),
        "no partial queue precedes the loss: {before:?}"
    );
    assert_eq!(recovery.loss, marker);
    assert_eq!(marker.subscription_id, *withheld.feed.id());
    assert_eq!(
        (marker.from_seq, marker.cause),
        (
            EventSeq::new(first.ok_or("no commit")?),
            LossCause::QueueOverflow
        )
    );
    assert!(recovery.snapshot.cursor.seq.get() >= marker_seq);
    assert_eq!(
        recovery.snapshot.sections.get(&1).map(String::as_str),
        Some("three")
    );
    assert_eq!(
        after
            .iter()
            .map(|event| event.cursor.seq.get())
            .collect::<Vec<_>>(),
        vec![next]
    );
    drop((witness, withheld));
    world.finish()
}

#[test]
fn a_forced_close_marker_names_the_first_undelivered_event_and_reconnect_replays_it() -> TestResult
{
    // Given a consumer with two fanned-out but undelivered events.
    let world = World::open(Bounds::v1()?)?;
    let witness = world.subscribe(Start::Head, Selector::all())?;
    let closing = world.subscribe(Start::Snapshot, Selector::all())?;
    let cursor = closing
        .snapshot
        .as_ref()
        .ok_or("snapshot start carried no snapshot")?
        .cursor;
    let first = world.set(1, "one")?.seq.get();
    let second = world.set(2, "two")?.seq.get();
    scan_to(&witness.feed, second)?;
    // When the transport force-closes it without writing its final error.
    let id = closing.feed.id().clone();
    let record = closing
        .feed
        .close_forced()?
        .ok_or("undelivered events owe a marker")?;
    let back = world.subscribe(Start::Resume(cursor), Selector::all())?;
    let (marker_seq, _) = until_loss_event(&witness.feed, &[])?;
    let (replayed, _) = scan_to(&back.feed, marker_seq)?;
    // Then the marker names the first undelivered event and the reconnect replays it.
    assert_eq!(
        (
            record.subscription_id.clone(),
            record.from_seq,
            record.cause
        ),
        (id, EventSeq::new(first), LossCause::ForcedClose)
    );
    let seqs: Vec<u64> = replayed
        .iter()
        .map(|event| event.cursor.seq.get())
        .collect();
    assert_eq!(seqs, vec![first, second, marker_seq]);
    assert!(
        replayed
            .iter()
            .any(|event| event.data == Data::Lost(record.clone()))
    );
    drop((witness, back));
    world.finish()
}

#[test]
fn a_dropped_consumer_still_leaves_a_durable_marker() -> TestResult {
    // Given a consumer holding one undelivered event and a witness that saw it.
    let world = World::open(Bounds::v1()?)?;
    let witness = world.subscribe(Start::Head, Selector::all())?;
    let dropped = world.subscribe(Start::Head, Selector::all())?;
    let seq = world.set(1, "one")?.seq.get();
    scan_to(&witness.feed, seq)?;
    let id = dropped.feed.id().clone();
    // When its connection vanishes and no further commit follows.
    drop(dropped);
    let (_, marker) = until_loss_event(&witness.feed, &[])?;
    // Then the worker committed the forced-close marker on its own.
    assert_eq!(
        (marker.subscription_id, marker.from_seq, marker.cause),
        (id, EventSeq::new(seq), LossCause::ForcedClose)
    );
    drop(witness);
    world.finish()
}

#[test]
fn the_session_budget_evicts_the_largest_queue_not_the_draining_one() -> TestResult {
    // Given a session budget of a few events, a draining consumer and a withheld one.
    let mut bounds = Bounds::v1()?;
    bounds.queue.session_bytes = 1_500;
    let world = World::open(bounds)?;
    let draining = world.subscribe(Start::Head, Selector::all().terminals([terminal(1)?]))?;
    let withheld = world.subscribe(Start::Head, Selector::all())?;
    // When changes keep committing and only the draining consumer reads.
    let mut delivered = Vec::new();
    let mut committed = Vec::new();
    for round in 0..20 {
        let seq = world.set(1, &format!("v{round}"))?.seq.get();
        committed.push(seq);
        delivered.extend(scan_to(&draining.feed, seq)?.0);
    }
    let (_, recovery) = until_missed(&withheld.feed)?;
    // Then the withheld queue was evicted and the draining consumer lost nothing.
    assert_eq!(recovery.loss.cause, LossCause::SessionBudget);
    assert_eq!(
        delivered
            .iter()
            .map(|event| event.cursor.seq.get())
            .collect::<Vec<_>>(),
        committed
    );
    drop((draining, withheld));
    world.finish()
}

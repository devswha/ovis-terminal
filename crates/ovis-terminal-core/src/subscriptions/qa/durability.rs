//! Loss markers are durable before the consumer hears of the loss or the close returns,
//! proven while the fan-out worker is held so its idle tick cannot commit them instead.

use std::error::Error;
use std::sync::mpsc::TryRecvError;

use ovis_terminal_core::events::{Cursor, EventError};
use ovis_terminal_core::subscriptions::{LossCause, LossRecord, Pulled, Selector, Start};
use ovis_terminal_protocol::schema::scalar::EventSeq;

use super::fixture::{Bounds, Data, TestResult, World};
use super::pulls::{pull, scan_to};

/// Loss markers committed after `cursor`, read straight from the durable log.
fn durable_markers(world: &World, cursor: Cursor) -> Result<Vec<LossRecord>, Box<dyn Error>> {
    let replay = world.core.subscribe(Some(cursor))?;
    let mut markers = Vec::new();
    loop {
        match replay.delivery.try_recv() {
            Ok(event) => {
                if let Data::Lost(record) = event.data {
                    markers.push(record);
                }
            }
            Err(EventError::TryReceive(TryRecvError::Empty)) => return Ok(markers),
            Err(other) => return Err(other.into()),
        }
    }
}

#[test]
fn close_forced_returns_only_after_its_marker_is_durable() -> TestResult {
    // Given a consumer with two undelivered events and a fan-out worker held mid-delivery.
    let world = World::open(Bounds::v1()?)?;
    let witness = world.subscribe(Start::Head, Selector::all())?;
    let closing = world.subscribe(Start::Snapshot, Selector::all())?;
    let cursor = closing
        .snapshot
        .as_ref()
        .ok_or("snapshot start carried no snapshot")?
        .cursor;
    let id = closing.feed.id().clone();
    world.set(1, "one")?;
    let second = world.set(2, "two")?.seq.get();
    scan_to(&witness.feed, second)?;
    let release = world.hold_worker()?;
    // When the transport force-closes it while the worker cannot commit anything.
    let record = closing
        .feed
        .close_forced()?
        .ok_or("undelivered events owe a marker")?;
    let durable = durable_markers(&world, cursor)?;
    release.send(())?;
    // Then the marker it returned is already in the durable log.
    assert_eq!(
        (record.subscription_id.clone(), record.cause),
        (id, LossCause::ForcedClose)
    );
    assert_eq!(
        durable,
        vec![record],
        "marker not durable at close_forced return"
    );
    drop(witness);
    world.finish()
}

#[test]
fn missed_is_returned_only_after_its_marker_is_durable() -> TestResult {
    // Given three committed changes, a two-event queue bound and a held fan-out worker.
    let mut bounds = Bounds::v1()?;
    bounds.queue.queue_events = 2;
    let world = World::open(bounds)?;
    let origin = world.fresh()?.0;
    let start = world.set(1, "zero")?;
    let cursor = Cursor::new(start.machine_id, start.runtime_id, EventSeq::new(origin));
    for value in ["one", "two"] {
        world.set(1, value)?;
    }
    let release = world.hold_worker()?;
    // When a consumer resumes with a replay larger than its queue, so the loss is marked
    // by the subscribing thread and only the consumer itself can commit the marker.
    let resumed = world.subscribe(Start::Resume(cursor), Selector::all())?;
    let pulled = pull(&resumed.feed)?;
    let durable = durable_markers(&world, cursor)?;
    release.send(())?;
    // Then Missed carries the loss and that marker was committed before Missed returned.
    let Pulled::Missed(recovery) = pulled else {
        return Err(format!("expected a loss, got {pulled:?}").into());
    };
    assert_eq!(recovery.loss.subscription_id, *resumed.feed.id());
    assert_eq!(recovery.loss.cause, LossCause::QueueOverflow);
    assert_eq!(
        durable,
        vec![recovery.loss],
        "marker not durable when Missed returned"
    );
    drop(resumed);
    world.finish()
}

use std::sync::mpsc::sync_channel;

use ovis_terminal_core::subscriptions::{LossCause, Selector, Start};

use super::fixture::{BOUND, Bounds, PUMP_GATE, TestResult, World, terminal};
use super::pulls::{scan_to, until_missed};

#[test]
fn a_stalled_fanout_worker_loses_every_subscription_and_each_recovers() -> TestResult {
    // Given a two-event coordinator delivery bound and two subscriptions.
    let mut bounds = Bounds::v1()?;
    bounds.core.queued_events = 2;
    let world = World::open(bounds)?;
    let all = world.subscribe(Start::Head, Selector::all())?;
    let second = world.subscribe(Start::Head, Selector::all().terminals([terminal(2)?]))?;
    // When the worker is held inside one fan-out while three more changes commit.
    let (entered, reached) = sync_channel(1);
    let (release, released) = sync_channel(1);
    *PUMP_GATE.lock().map_err(|_| "gate poisoned")? = Some((entered, released));
    world.gate()?;
    reached.recv_timeout(BOUND)?;
    for value in ["x", "y", "w"] {
        world.set(1, value)?;
    }
    release.send(())?;
    let (_, lost_all) = until_missed(&all.feed)?;
    let (_, lost_second) = until_missed(&second.feed)?;
    let next = world.set(2, "z")?.seq.get();
    let (after_all, _) = scan_to(&all.feed, next)?;
    let (after_second, _) = scan_to(&second.feed, next)?;
    // Then both recover from durable markers with a snapshot that includes the overflowed
    // changes, and delivery continues after it.
    for recovery in [&lost_all, &lost_second] {
        assert_eq!(recovery.loss.cause, LossCause::FanoutOverflow);
        assert_eq!(
            recovery.snapshot.sections.get(&1).map(String::as_str),
            Some("w")
        );
    }
    for after in [&after_all, &after_second] {
        let seqs: Vec<u64> = after.iter().map(|event| event.cursor.seq.get()).collect();
        assert_eq!(seqs, vec![next]);
    }
    drop((all, second));
    world.finish()
}

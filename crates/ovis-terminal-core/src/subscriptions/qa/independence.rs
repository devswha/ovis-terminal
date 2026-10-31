use std::time::{Duration, Instant};

use ovis_terminal_core::subscriptions::{
    MAX_HEARTBEAT, Pulled, QueueLimits, Request, Selector, Start,
};
use ovis_terminal_protocol::schema::scalar::EventSeq;

use super::fixture::{BOUND, Bounds, TestResult, World, identity, terminal};
use super::pulls::{pull, scan_to};

#[test]
fn unsubscribing_one_subscription_leaves_the_others_delivering() -> TestResult {
    // Given two subscriptions that have both scanned one commit.
    let world = World::open(Bounds::v1()?)?;
    let kept = world.subscribe(Start::Head, Selector::all())?;
    let closed = world.subscribe(Start::Head, Selector::all())?;
    let first = world.set(1, "one")?.seq.get();
    scan_to(&kept.feed, first)?;
    scan_to(&closed.feed, first)?;
    // When one is unsubscribed by identity and another commit follows.
    let position = world.hub.unsubscribe(closed.feed.id())?;
    let second = world.set(1, "two")?.seq.get();
    let (events, _) = scan_to(&kept.feed, second)?;
    // Then the closed consumer reads where it stopped, the other keeps delivering, and
    // the closed identity is gone for every later call.
    assert_eq!(
        (position.seq, position.scan_seq),
        (EventSeq::new(first), EventSeq::new(first))
    );
    assert!(matches!(pull(&closed.feed)?, Pulled::Closed(at) if at == position));
    assert_eq!(
        events
            .iter()
            .map(|event| event.cursor.seq.get())
            .collect::<Vec<_>>(),
        vec![second]
    );
    let again = world.hub.unsubscribe(closed.feed.id());
    let update = world.hub.update(closed.feed.id(), Selector::all());
    assert!(matches!(again, Err(error) if error.kind() == "not_found"));
    assert!(matches!(update, Err(error) if error.kind() == "not_found"));
    drop((kept, closed));
    world.finish()
}

#[test]
fn the_subscription_limit_refuses_without_disturbing_installed_ones() -> TestResult {
    // Given a hub admitting two subscriptions, both installed.
    let mut bounds = Bounds::v1()?;
    bounds.queue.subscriptions = 2;
    let world = World::open(bounds)?;
    let first = world.subscribe(Start::Head, Selector::all())?;
    let second = world.subscribe(Start::Head, Selector::all())?;
    // When a third subscribes, and after one consumer goes away, a fourth.
    let refused = world
        .hub
        .subscribe(Request::new(Start::Snapshot, Selector::all()));
    let seq = world.set(1, "one")?.seq.get();
    let (events, _) = scan_to(&first.feed, seq)?;
    scan_to(&second.feed, seq)?;
    drop(second);
    let admitted = world.subscribe(Start::Head, Selector::all());
    // Then only the over-limit request is refused and installed delivery is untouched.
    assert!(matches!(refused, Err(error) if error.kind() == "subscription_limit"));
    assert_eq!(events.len(), 1);
    assert!(admitted.is_ok(), "{admitted:?}");
    drop((first, admitted));
    world.finish()
}

#[test]
fn an_idle_filtered_subscription_heartbeats_its_scan_cursor_and_identity() -> TestResult {
    // Given a subscription to terminal 1 and an unfiltered witness started at the same head.
    let world = World::open(Bounds::v1()?)?;
    let filtered = world.subscribe(Start::Head, Selector::all().terminals([terminal(1)?]))?;
    let witness = world.subscribe(Start::Head, Selector::all())?;
    // When only terminal 2 changes and the witness has seen it fanned out.
    let seq = world.set(2, "elsewhere")?.seq.get();
    scan_to(&witness.feed, seq)?;
    let pulled = filtered.feed.next(Instant::now())?;
    // Then the idle subscription reports nothing delivered but the commit as scanned,
    // with the runtime identity that owns the sequence.
    let Pulled::Heartbeat(beat) = pulled else {
        return Err(format!("expected a heartbeat, got {pulled:?}").into());
    };
    let runtime = identity(1)?;
    assert_eq!(beat.subscription_id, *filtered.feed.id());
    assert_eq!(
        (beat.seq, beat.scan_seq),
        (filtered.snapshot_seq, EventSeq::new(seq))
    );
    assert_eq!(
        (beat.machine_id, beat.runtime_id, beat.boot_id),
        (
            runtime.machine_id(),
            runtime.runtime_id(),
            runtime.server_boot_id()
        )
    );
    drop((filtered, witness));
    world.finish()
}

#[test]
fn heartbeat_bounds_never_exceed_fifteen_seconds() -> TestResult {
    // Given the advertised bounds and two invalid variants.
    let advertised = QueueLimits::v1()?;
    let mut slow = advertised;
    slow.heartbeat = MAX_HEARTBEAT + Duration::from_millis(1);
    let mut unbounded = advertised;
    unbounded.queue_events = 0;
    // When each is validated.
    let checked = (advertised.checked(), slow.checked(), unbounded.checked());
    // Then only the advertised bounds pass, and they heartbeat within 15 seconds.
    assert!(advertised.heartbeat <= MAX_HEARTBEAT && advertised.heartbeat > Duration::ZERO);
    assert!(checked.0.is_ok());
    assert!(matches!(checked.1, Err(error) if error.kind() == "invalid_params"));
    assert!(matches!(checked.2, Err(error) if error.kind() == "invalid_params"));
    Ok(())
}

#[test]
fn an_idle_subscription_heartbeats_within_its_interval_whatever_the_caller_waits() -> TestResult {
    // Given a hub whose heartbeat interval is far shorter than the caller's wait.
    let mut bounds = Bounds::v1()?;
    bounds.queue.heartbeat = Duration::from_millis(50);
    let world = World::open(bounds)?;
    let idle = world.subscribe(Start::Head, Selector::all())?;
    let caller_wait = Instant::now()
        .checked_add(Duration::from_secs(3600))
        .ok_or("deadline overflows")?;
    // When the consumer waits on an idle subscription.
    let started = Instant::now();
    let pulled = idle.feed.next(caller_wait)?;
    // Then the heartbeat arrives at the interval, not at the caller's deadline.
    assert!(matches!(pulled, Pulled::Heartbeat(_)), "{pulled:?}");
    assert!(started.elapsed() < BOUND, "waited {:?}", started.elapsed());
    drop(idle);
    world.finish()
}

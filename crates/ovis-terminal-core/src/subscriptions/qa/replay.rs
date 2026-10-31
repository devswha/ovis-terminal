use std::sync::Arc;
use std::sync::mpsc::sync_channel;

use ovis_terminal_core::events::{Cursor, MissedReason};
use ovis_terminal_core::subscriptions::{Selector, Start};
use ovis_terminal_protocol::schema::scalar::EventSeq;

use super::fixture::{BOUND, Bounds, TestResult, World, apply};
use super::pulls::scan_to;

#[test]
fn a_snapshot_racing_commits_is_followed_by_exactly_the_later_commits() -> TestResult {
    // Given a writer committing fifty changes on another thread.
    let world = Arc::new(World::open(Bounds::v1()?)?);
    let (started, start) = sync_channel(1);
    let writer = {
        let world = Arc::clone(&world);
        std::thread::spawn(move || -> Result<u64, String> {
            started.send(()).map_err(|error| error.to_string())?;
            let mut last = 0;
            for round in 0..50_u64 {
                last = world
                    .set(1 + round % 3, &format!("v{round}"))
                    .map_err(|error| error.to_string())?
                    .seq
                    .get();
            }
            Ok(last)
        })
    };
    // When a snapshot subscription starts while it runs.
    start.recv_timeout(BOUND)?;
    let subscribed = world.subscribe(Start::Snapshot, Selector::all())?;
    let last = writer.join().map_err(|_| "writer panicked")??;
    let snapshot = subscribed
        .snapshot
        .clone()
        .ok_or("snapshot start carried no snapshot")?;
    let (events, _) = scan_to(&subscribed.feed, last)?;
    // Then delivery is exactly the commits after R, contiguous, and rebuilds fresh state.
    let seqs: Vec<u64> = events.iter().map(|event| event.cursor.seq.get()).collect();
    let after = snapshot.cursor.seq.get();
    assert_eq!(seqs, (after + 1..=last).collect::<Vec<_>>());
    let mut rebuilt = snapshot.sections;
    apply(&mut rebuilt, &events);
    assert_eq!(world.fresh()?, (last, rebuilt));
    drop(subscribed);
    Arc::try_unwrap(world)
        .map_err(|_| "world still shared")?
        .finish()
}

#[test]
fn an_expired_cursor_misses_with_a_fresh_snapshot_and_the_floor_still_resumes() -> TestResult {
    // Given a store that retains only the last two events, after five commits.
    let mut bounds = Bounds::v1()?;
    bounds.store = bounds.store.with_max_events(2);
    let world = World::open(bounds)?;
    let first = world.set(1, "one")?;
    let mut last = first;
    for value in ["two", "three", "four", "five"] {
        last = world.set(2, value)?;
    }
    // When one consumer resumes below the floor and another exactly at it.
    let expired = world.subscribe(Start::Resume(first), Selector::all())?;
    let floor = expired.replay_floor;
    let at_floor = Cursor::new(first.machine_id, first.runtime_id, floor);
    let resumed = world.subscribe(Start::Resume(at_floor), Selector::all())?;
    let (replayed, _) = scan_to(&resumed.feed, last.seq.get())?;
    // Then the first gets cursor_expired with a complete fresh snapshot at the head, and
    // the floor cursor replays the retained events without a snapshot.
    assert_eq!(expired.missed, Some(MissedReason::CursorExpired));
    assert_eq!(floor, EventSeq::new(last.seq.get() - 2));
    let snapshot = expired
        .snapshot
        .ok_or("expired cursor carried no snapshot")?;
    assert_eq!(
        (snapshot.cursor.seq, expired.snapshot_seq),
        (last.seq, last.seq)
    );
    assert_eq!(world.fresh()?, (last.seq.get(), snapshot.sections));
    assert_eq!((resumed.missed, resumed.snapshot.is_none()), (None, true));
    let seqs: Vec<u64> = replayed
        .iter()
        .map(|event| event.cursor.seq.get())
        .collect();
    assert_eq!(seqs, vec![last.seq.get() - 1, last.seq.get()]);
    drop((expired.feed, resumed));
    world.finish()
}

#[test]
fn a_cursor_ahead_of_the_head_installs_nothing() -> TestResult {
    // Given a hub admitting one subscription and one committed change.
    let mut bounds = Bounds::v1()?;
    bounds.queue.subscriptions = 1;
    let world = World::open(bounds)?;
    let head = world.set(1, "one")?;
    // When a cursor past the head is presented, then a valid subscription.
    let ahead = Cursor::new(
        head.machine_id,
        head.runtime_id,
        EventSeq::new(head.seq.get() + 1),
    );
    let refused = world.subscribe(Start::Resume(ahead), Selector::all());
    let admitted = world.subscribe(Start::Resume(head), Selector::all());
    // Then the refusal left the only slot free.
    assert!(refused.is_err());
    assert!(admitted.is_ok(), "{admitted:?}");
    drop(admitted);
    world.finish()
}

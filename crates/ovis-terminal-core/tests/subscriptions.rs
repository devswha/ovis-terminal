//! Independent subscriptions, replay and recoverable loss against the real SQLite writer.

#[path = "../src/subscriptions/qa/durability.rs"]
mod durability;
#[path = "../src/subscriptions/qa/fixture.rs"]
pub mod fixture;
#[path = "../src/subscriptions/qa/health.rs"]
mod health;
#[path = "../src/subscriptions/qa/independence.rs"]
mod independence;
#[path = "../src/subscriptions/qa/loss.rs"]
mod loss;
#[path = "../src/subscriptions/qa/pulls.rs"]
pub mod pulls;
#[path = "../src/subscriptions/qa/replay.rs"]
mod replay;
#[path = "../src/subscriptions/qa/stall.rs"]
mod stall;
#[path = "../src/subscriptions/qa/wire.rs"]
mod wire;

use fixture::{Bounds, Data, TestResult, World, apply, terminal};
use ovis_terminal_core::events::{Cursor, Event, MissedReason};
use ovis_terminal_core::registry::RuntimeId;
use ovis_terminal_core::subscriptions::{LossCause, Request, Selector, Start};
use ovis_terminal_protocol::schema::scalar::EventSeq;
use pulls::{scan_to, until_loss_event};

fn seqs(events: &[Event<Data>]) -> Vec<u64> {
    events.iter().map(|event| event.cursor.seq.get()).collect()
}

#[test]
fn happy() -> TestResult {
    // Given two subscriptions on one hub, the second filtered to terminal 1.
    let world = World::open(Bounds::v1()?)?;
    let all = world.subscribe(Start::Snapshot, Selector::all())?;
    let filtered = world.subscribe(Start::Snapshot, Selector::all().terminals([terminal(1)?]))?;
    let base = all
        .snapshot
        .clone()
        .ok_or("snapshot start carried no snapshot")?;
    // When terminal 2 then terminal 1 change, the filter is replaced by terminal 2, and
    // both terminals change again.
    let first = world.set(2, "b")?.seq.get();
    let second = world.set(1, "a")?.seq.get();
    let (all_before, _) = scan_to(&all.feed, second)?;
    let (filtered_before, scanned) = scan_to(&filtered.feed, second)?;
    let effective = world.hub.update(
        filtered.feed.id(),
        Selector::all().terminals([terminal(2)?]),
    )?;
    let third = world.set(1, "a2")?.seq.get();
    let fourth = world.set(2, "b2")?.seq.get();
    let (all_after, _) = scan_to(&all.feed, fourth)?;
    let (filtered_after, _) = scan_to(&filtered.feed, fourth)?;
    // Then both streams continue across the replacement, the filtered gap is scanned
    // rather than lost, and snapshot plus events equals a fresh snapshot.
    assert_eq!(seqs(&all_before), vec![first, second]);
    assert_eq!(seqs(&filtered_before), vec![second]);
    assert!(
        scanned >= second,
        "nonmatching {first} is covered by scan_seq {scanned}"
    );
    assert_eq!(effective, EventSeq::new(second + 1));
    assert_eq!(seqs(&all_after), vec![third, fourth]);
    assert_eq!(seqs(&filtered_after), vec![fourth]);
    let mut rebuilt = base.sections.clone();
    apply(&mut rebuilt, &all_before);
    apply(&mut rebuilt, &all_after);
    let (fresh_seq, fresh) = world.fresh()?;
    assert_eq!((fresh_seq, &fresh), (fourth, &rebuilt));
    // And a reconnect from the snapshot cursor replays the identical events.
    let resumed = world.subscribe(Start::Resume(base.cursor), Selector::all())?;
    assert_eq!((resumed.missed, resumed.snapshot.is_none()), (None, true));
    let (replayed, _) = scan_to(&resumed.feed, fourth)?;
    assert_eq!(replayed, [all_before, all_after].concat());
    println!(
        "{}",
        serde_json::json!({
            "subscriptions": [all.feed.id(), filtered.feed.id()],
            "effective_seq": effective,
            "filtered_scan_seq": scanned.to_string(),
            "replayed_seqs": seqs(&replayed).iter().map(u64::to_string).collect::<Vec<_>>(),
            "rebuilt_equals_fresh": rebuilt == fresh,
        })
    );
    drop((all, filtered, resumed));
    world.finish()
}

#[test]
fn failure() -> TestResult {
    // Given a consumer that withholds reads past its two-event bound beside a healthy one,
    // on a store that retains only the last three events.
    let mut bounds = Bounds::v1()?;
    bounds.queue.queue_events = 2;
    bounds.store = bounds.store.with_max_events(3);
    let world = World::open(bounds)?;
    let healthy = world.subscribe(Start::Head, Selector::all())?;
    let withheld = world.subscribe(Start::Snapshot, Selector::all())?;
    let cursor = withheld
        .snapshot
        .as_ref()
        .ok_or("snapshot start carried no snapshot")?
        .cursor;
    let lost_id = withheld.feed.id().clone();
    let mut committed = Vec::new();
    let mut seen = Vec::new();
    for value in ["one", "two", "three"] {
        let seq = world.set(1, value)?.seq.get();
        seen.extend(scan_to(&healthy.feed, seq)?.0);
        committed.push(seq);
    }
    let (marker_seq, marker) = until_loss_event(&healthy.feed, &seen)?;
    // When its final error is never read: the connection drops and it reconnects from
    // its last cursor after that cursor left retention.
    drop(withheld);
    let back = world.subscribe(Start::Resume(cursor), Selector::all())?;
    let next = world.set(2, "after")?.seq.get();
    let (after, _) = scan_to(&back.feed, next)?;
    // Then the healthy consumer saw every event and the committed loss marker, and the
    // reconnect gets missed plus a fresh snapshot, then live delivery after it.
    assert_eq!(marker.subscription_id, lost_id);
    assert_eq!(Some(&marker.from_seq.get()), committed.first());
    assert_eq!(marker.cause, LossCause::QueueOverflow);
    assert_eq!(back.missed, Some(MissedReason::CursorExpired));
    let snapshot = back
        .snapshot
        .as_ref()
        .ok_or("expired cursor carried no snapshot")?;
    assert_eq!(snapshot.cursor.seq.get(), marker_seq);
    assert_eq!(snapshot.sections.get(&1).map(String::as_str), Some("three"));
    assert_eq!(seqs(&after), vec![next]);
    // And a foreign lifetime is a typed miss with a fresh snapshot, an ahead cursor a
    // typed rejection.
    let foreign = Cursor::new(cursor.machine_id, RuntimeId::try_from(2)?, cursor.seq);
    let mismatched = world.subscribe(Start::Resume(foreign), Selector::all())?;
    assert_eq!(mismatched.missed, Some(MissedReason::LifetimeMismatch));
    let (_, fresh) = world.fresh()?;
    assert_eq!(
        mismatched.snapshot.map(|snapshot| snapshot.sections),
        Some(fresh)
    );
    let ahead = Cursor::new(
        cursor.machine_id,
        cursor.runtime_id,
        EventSeq::new(next + 10),
    );
    let refused = world
        .hub
        .subscribe(Request::new(Start::Resume(ahead), Selector::all()));
    assert!(
        matches!(&refused, Err(error) if error.kind() == "seq_ahead"),
        "{refused:?}"
    );
    println!(
        "{}",
        serde_json::json!({
            "loss": marker,
            "loss_marker_seq": marker_seq.to_string(),
            "reconnect_missed": back.missed,
            "reconnect_snapshot_seq": snapshot.cursor.seq,
            "foreign_missed": mismatched.missed,
        })
    );
    drop((healthy, back, mismatched.feed));
    world.finish()
}

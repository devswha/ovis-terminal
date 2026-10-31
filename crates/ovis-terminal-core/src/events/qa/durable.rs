//! Real store recovery, mirrored ordering and pinned-page behavior.

use std::sync::mpsc::TryRecvError;

use ovis_terminal_core::events::{
    Event, Limits, RemoteOrigin, RemoteSeq, StateSnapshot, SubscribeRequest,
};
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_store::StoreLimits;

use super::{
    ByteOffset, Changed, Coordinator, EventError, Fixture, MachineId, MissedReason, Producer,
    RuntimeId, Section, SnapshotSections, StoreOptions, TestResult, Uuid, change, identity,
};

#[test]
fn disconnect_replays_commits_across_core_restart_without_renumbering_origin() -> TestResult {
    // Given a disconnected subscription after a snapshot boundary.
    let fixture = Fixture::new();
    let core = Coordinator::new(
        fixture.open(StoreOptions::default())?,
        identity(1)?,
        Producer,
    );
    let subscription = core.subscribe(None)?;
    let cursor = subscription
        .snapshot
        .as_ref()
        .ok_or("snapshot missing")?
        .cursor;
    drop(subscription);
    core.commit(change("one", 0)?)?;
    let origin = RemoteOrigin::new(
        MachineId::new(Uuid::from_u128(2))?,
        RuntimeId::try_from(9)?,
        RemoteSeq::new(EventSeq::new(u64::MAX)),
    );
    let last = core.commit(change("two", 1)?.mirrored_from(origin.clone()))?;
    let old_page = core.pin_snapshot()?;
    core.close()?;
    // When a new core boot reopens the real store and resumes the retained cursor.
    let restarted = Coordinator::<Changed, _>::new(
        fixture.open(StoreOptions::default())?,
        identity(2)?,
        Producer,
    );
    let resumed = restarted.subscribe(Some(cursor))?;
    assert!(
        matches!(restarted.page(&old_page.token, ByteOffset::new(0)),
        Err(error) if error.kind() == "stale_snapshot")
    );
    // Then replay preserves commit order, original boot and remote sequence independently.
    assert!(resumed.snapshot.is_none());
    let first = resumed.delivery.try_recv()?;
    let second = resumed.delivery.try_recv()?;
    assert_eq!(first.data.value, "one");
    assert_eq!(second.data.value, "two");
    assert!(first.cursor.seq < second.cursor.seq);
    assert_eq!(second.cursor, last);
    assert_eq!(second.boot_id, identity(1)?.server_boot_id());
    assert_eq!(second.origin, Some(origin));
    assert!(matches!(
        resumed.delivery.try_recv(),
        Err(EventError::TryReceive(TryRecvError::Empty))
    ));
    let next = restarted.commit(change("three", 2)?)?;
    let third = resumed.delivery.try_recv()?;
    assert!(next.seq > last.seq);
    assert_eq!(third.cursor, next);
    assert_eq!(third.boot_id, identity(2)?.server_boot_id());
    let bytes = serde_json::to_vec(&third)?;
    assert_eq!(serde_json::from_slice::<Event<Changed>>(&bytes)?, third);
    restarted.close()?;
    Ok(())
}

#[test]
fn paged_subscription_pins_revision_until_release_across_concurrent_mutation() -> TestResult {
    // Given an atomic paged installation with deliberately small pages.
    let fixture = Fixture::new();
    let mut limits = Limits::default();
    limits.page_bytes = 23;
    let core = Coordinator::new(
        fixture.open(StoreOptions::default())?,
        identity(1)?,
        Producer,
    )
    .with_limits(limits)?;
    core.commit(change("one", 0)?)?;
    let subscription = core.subscribe_paged(SubscribeRequest::default())?;
    let lease = subscription.snapshot.ok_or("snapshot lease missing")?;
    // When a mutation commits before the remaining pages are read.
    let changed = core.commit(change("two", 1)?)?;
    let mut bytes = Vec::new();
    let mut offset = ByteOffset::new(0);
    loop {
        let page = core.page(&lease.token, offset)?;
        assert_eq!(page.cursor, lease.cursor);
        assert_eq!(page.total, lease.total);
        assert!(page.data.len() <= 23);
        bytes.extend_from_slice(&page.data);
        match page.next_offset {
            Some(next) => offset = next,
            None => break,
        }
    }
    // Then reconstruction sees the old state and exactly one later event.
    let snapshot: StateSnapshot<SnapshotSections<(), Vec<String>>> =
        serde_json::from_slice(&bytes)?;
    assert_eq!(
        snapshot.sections.terminals,
        Section::Complete(vec!["one".into()])
    );
    assert_eq!(u64::try_from(bytes.len())?, lease.total.get());
    assert_eq!(subscription.delivery.try_recv()?.cursor, changed);
    assert!(matches!(
        subscription.delivery.try_recv(),
        Err(EventError::TryReceive(TryRecvError::Empty))
    ));
    assert!(matches!(core.page(&lease.token, ByteOffset::new(u64::MAX)),
        Err(error) if error.kind() == "invalid_params"));
    core.release_snapshot(&lease.token)?;
    assert!(matches!(core.page(&lease.token, ByteOffset::new(0)),
        Err(error) if error.kind() == "stale_snapshot"));
    core.close()?;
    Ok(())
}

#[test]
fn expired_cursor_gets_fresh_state_instead_of_a_truncated_replay() -> TestResult {
    // Given retention that evicts the first of two committed events.
    let fixture = Fixture::new();
    let options = StoreOptions::default().with_limits(StoreLimits::default().with_max_events(1));
    let core = Coordinator::new(fixture.open(options)?, identity(1)?, Producer);
    let cursor = core
        .subscribe(None)?
        .snapshot
        .ok_or("snapshot missing")?
        .cursor;
    core.commit(change("one", 0)?)?;
    let last = core.commit(change("two", 1)?)?;
    // When the old cursor resumes.
    let resumed = core.subscribe(Some(cursor))?;
    // Then the typed retention rejection accompanies the latest state.
    assert_eq!(resumed.missed, Some(MissedReason::CursorExpired));
    let snapshot = resumed.snapshot.ok_or("fresh snapshot missing")?;
    assert_eq!(snapshot.cursor, last);
    assert_eq!(
        snapshot.sections.terminals,
        Section::Complete(vec!["two".into()])
    );
    assert!(matches!(
        resumed.delivery.try_recv(),
        Err(EventError::TryReceive(TryRecvError::Empty))
    ));
    core.close()?;
    Ok(())
}

#[test]
fn snapshot_larger_than_one_frame_reconstructs_from_bounded_pages() -> TestResult {
    // Given a logical snapshot exceeding the 16 MiB transport frame bound.
    let fixture = Fixture::new();
    let options =
        StoreOptions::default().with_limits(StoreLimits::default().with_max_value_bytes(32 << 20));
    let core = Coordinator::new(fixture.open(options)?, identity(1)?, Producer);
    let input = "x".repeat(17 << 20);
    core.commit(change(&input, 0)?)?;
    let subscription = core.subscribe_paged(SubscribeRequest::default())?;
    let lease = subscription.snapshot.ok_or("snapshot lease missing")?;
    assert!(lease.total.get() > 16 << 20);
    // When all pages are assembled using the returned continuation offsets.
    let mut bytes = Vec::new();
    let mut offset = ByteOffset::new(0);
    loop {
        let page = core.page(&lease.token, offset)?;
        assert_eq!(page.cursor, lease.cursor);
        assert!(page.data.len() <= 64 << 10);
        bytes.extend_from_slice(&page.data);
        match page.next_offset {
            Some(next) => offset = next,
            None => break,
        }
    }
    // Then a complete typed record survives without truncation or changed revision.
    let snapshot: StateSnapshot<SnapshotSections<(), Vec<String>>> =
        serde_json::from_slice(&bytes)?;
    assert_eq!(snapshot.cursor, lease.cursor);
    assert_eq!(snapshot.sections.terminals, Section::Complete(vec![input]));
    core.release_snapshot(&lease.token)?;
    core.close()?;
    Ok(())
}

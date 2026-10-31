//! Wire round trips and independent subscription contracts.

use std::sync::mpsc::TryRecvError;

use ovis_terminal_core::events::{Cursor, Limits, ResourceId, SubscribeRequest};
use ovis_terminal_protocol::schema::scalar::EventSeq;
use proptest::prelude::*;

use super::{
    Changed, Coordinator, EventError, Filter, Fixture, Producer, Section, StoreOptions, TerminalId,
    TestResult, change, identity,
};

#[test]
fn maximum_sequence_round_trips_exactly_and_numeric_forms_reject() -> TestResult {
    // Given the entire unsigned sequence range, beyond JavaScript's exact numbers.
    let scope = identity(1)?;
    let cursor = Cursor::new(
        scope.machine_id(),
        scope.runtime_id(),
        EventSeq::new(u64::MAX),
    );
    // When it crosses the JSON boundary.
    let bytes = serde_json::to_vec(&cursor)?;
    let decoded = Cursor::decode(&bytes)?;
    // Then every bit is preserved and numeric form is a typed invalid_params.
    assert_eq!(decoded.seq.get(), u64::MAX);
    let mut json = serde_json::from_slice::<serde_json::Value>(&bytes)?;
    assert_eq!(
        json.get("seq"),
        Some(&serde_json::json!("18446744073709551615"))
    );
    *json.get_mut("seq").ok_or("sequence missing")? = serde_json::json!(u64::MAX);
    assert!(
        matches!(Cursor::decode(&serde_json::to_vec(&json)?), Err(error)
        if error.kind() == "invalid_params")
    );
    for invalid in ["01", "+1", "-1", " 1", "18446744073709551616"] {
        *json.get_mut("seq").ok_or("sequence missing")? = serde_json::json!(invalid);
        assert!(
            matches!(Cursor::decode(&serde_json::to_vec(&json)?), Err(error)
            if error.kind() == "invalid_params")
        );
    }
    Ok(())
}

proptest! {
    #[test]
    fn every_sequence_round_trips(sequence in any::<u64>()) {
        // Given any unsigned sequence in a valid lifetime.
        let scope = identity(1)?;
        let cursor = Cursor::new(scope.machine_id(), scope.runtime_id(), EventSeq::new(sequence));
        // When serialized and parsed at the boundary.
        let encoded = serde_json::to_vec(&cursor)?;
        let decoded = Cursor::decode(&encoded)?;
        // Then the original cursor and string counter survive.
        prop_assert_eq!(decoded, cursor);
        let value: serde_json::Value = serde_json::from_slice(&encoded)?;
        prop_assert_eq!(value.get("seq"), Some(&serde_json::json!(sequence.to_string())));
    }

    #[test]
    fn durable_records_round_trip(value in "[a-z]{1,32}") {
        // Given a committed typed mutation in the real SQLite store.
        let fixture = Fixture::new();
        let core = Coordinator::new(fixture.open(StoreOptions::default())?, identity(1)?, Producer);
        let cursor = core.subscribe(None)?.snapshot
            .ok_or_else(|| TestCaseError::fail("snapshot missing"))?.cursor;
        core.commit(change(&value, 0).map_err(|error| TestCaseError::fail(error.to_string()))?)?;
        core.close()?;
        // When a replacement core reads its durable event and state.
        let recovered = Coordinator::<Changed, _>::new(
            fixture.open(StoreOptions::default())?, identity(2)?, Producer,
        );
        let replay = recovered.subscribe(Some(cursor))?.delivery.try_recv()?;
        let snapshot = recovered.subscribe(None)?.snapshot
            .ok_or_else(|| TestCaseError::fail("snapshot missing"))?;
        // Then both independently recovered records preserve the original input.
        prop_assert_eq!(&replay.data.value, &value);
        prop_assert_eq!(snapshot.sections.terminals, Section::Complete(vec![value]));
        recovered.close()?;
    }
}

#[test]
fn independent_subscriptions_and_filters_survive_second_installation() -> TestResult {
    // Given one subscription and a second installation on the same coordinator.
    let fixture = Fixture::new();
    let core = Coordinator::new(
        fixture.open(StoreOptions::default())?,
        identity(1)?,
        Producer,
    );
    let first = core.subscribe(None)?;
    let second = core.subscribe(None)?;
    let other = core.subscribe_matching(SubscribeRequest::new(
        None,
        Filter::new(
            Vec::new(),
            vec![ResourceId::Terminal(TerminalId::try_from(2)?)],
        ),
    ))?;
    // When a matching mutation commits.
    let cursor = core.commit(change("one", 0)?)?;
    // Then both matching subscriptions receive exactly that commit independently.
    assert_ne!(first.id, second.id);
    assert_eq!(first.delivery.try_recv()?.cursor, cursor);
    assert_eq!(second.delivery.try_recv()?.cursor, cursor);
    assert!(matches!(
        other.delivery.try_recv(),
        Err(EventError::TryReceive(TryRecvError::Empty))
    ));
    assert!(matches!(
        first.delivery.try_recv(),
        Err(EventError::TryReceive(TryRecvError::Empty))
    ));
    assert!(matches!(
        second.delivery.try_recv(),
        Err(EventError::TryReceive(TryRecvError::Empty))
    ));
    core.close()?;
    Ok(())
}

#[test]
fn queue_overflow_is_visible_and_resumes_from_durable_cursor() -> TestResult {
    // Given a one-event delivery queue.
    let fixture = Fixture::new();
    let mut limits = Limits::default();
    limits.queued_events = 1;
    let core = Coordinator::new(
        fixture.open(StoreOptions::default())?,
        identity(1)?,
        Producer,
    )
    .with_limits(limits)?;
    let subscription = core.subscribe(None)?;
    // When the client withholds reads across two commits.
    core.commit(change("one", 0)?)?;
    let final_cursor = core.commit(change("two", 1)?)?;
    // Then no stale queued event is presented as a complete delivery stream.
    assert!(matches!(subscription.delivery.try_recv(), Err(error) if error.kind() == "missed"));
    let fresh = core
        .subscribe(None)?
        .snapshot
        .ok_or("fresh snapshot missing")?;
    assert_eq!(fresh.cursor, final_cursor);
    assert_eq!(
        fresh.sections.terminals,
        Section::Complete(vec!["two".into()])
    );
    core.close()?;
    Ok(())
}

#[test]
fn unsupported_sections_never_claim_empty_completeness() -> TestResult {
    // Given only a terminal producer.
    let fixture = Fixture::new();
    let core = Coordinator::<Changed, _>::new(
        fixture.open(StoreOptions::default())?,
        identity(1)?,
        Producer,
    );
    // When the complete implemented snapshot is captured.
    let snapshot = core.subscribe(None)?.snapshot.ok_or("snapshot missing")?;
    // Then missing producers remain explicitly unavailable.
    assert_eq!(snapshot.sections.layout, Section::Unavailable);
    assert_eq!(snapshot.sections.agents, Section::Unavailable);
    assert_eq!(snapshot.sections.services, Section::Unavailable);
    assert_eq!(snapshot.sections.terminals, Section::Complete(Vec::new()));
    core.close()?;
    Ok(())
}

#[test]
fn disconnected_queues_release_the_aggregate_byte_budget() -> TestResult {
    // Given a shared byte budget that fits only one copy of the event.
    let fixture = Fixture::new();
    let mut limits = Limits::default();
    limits.queued_bytes = 1700;
    let core = Coordinator::new(
        fixture.open(StoreOptions::default())?,
        identity(1)?,
        Producer,
    )
    .with_limits(limits)?;
    let stalled = core.subscribe(None)?;
    core.commit(change("one", 0)?)?;
    core.commit(change("two", 1)?)?;
    assert!(matches!(stalled.delivery.try_recv(), Err(error) if error.kind() == "missed"));
    // When that client disconnects and another snapshot installation starts.
    drop(stalled);
    let replacement = core.subscribe(None)?;
    let cursor = core.commit(change("three", 2)?)?;
    // Then queued-byte charges are reclaimed instead of starving later delivery.
    assert_eq!(replacement.delivery.try_recv()?.cursor, cursor);
    core.close()?;
    Ok(())
}

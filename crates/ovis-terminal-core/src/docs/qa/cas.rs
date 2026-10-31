//! Compare-and-swap, its event and its receipt commit together or not at all.

use std::sync::{Arc, Barrier};

use ovis_terminal_protocol::schema::methods::server::OpState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use ovis_terminal_store::StoreOptions;
use serde_json::{Value, json};

use super::fixture::{Core, Root, TestResult, at, failed_kind, get, op_minted, set};
use crate::docs::DocumentsRetentionEvent;
use crate::events::{Event, EventError};

fn drain(
    delivery: &crate::events::Delivery<DocumentsRetentionEvent>,
) -> Vec<Event<DocumentsRetentionEvent>> {
    std::iter::from_fn(|| delivery.try_recv().ok()).collect()
}

fn changes(events: &[Event<DocumentsRetentionEvent>]) -> Vec<&Event<DocumentsRetentionEvent>> {
    events
        .iter()
        .filter(|event| event.event_type == "doc.changed")
        .collect()
}

fn race(core: &Arc<Core>) -> TestResult<Vec<Value>> {
    let barrier = Arc::new(Barrier::new(2));
    let writers: Vec<_> = (0..2)
        .map(|writer| {
            let (core, barrier) = (Arc::clone(core), Arc::clone(&barrier));
            std::thread::spawn(move || -> Result<Value, String> {
                let op_id = op_minted(0).map_err(|error| error.to_string())?;
                barrier.wait();
                let params = json!({"name": "shared", "version": "0", "value": {"writer": writer}});
                let record = set(&core, &op_id, params).map_err(|error| error.to_string())?;
                serde_json::to_value(record.map_err(|error| error.kind().to_owned())?)
                    .map_err(|error| error.to_string())
            })
        })
        .collect();
    let mut records = Vec::new();
    for writer in writers {
        records.push(writer.join().map_err(|_| "writer panicked")??);
    }
    Ok(records)
}

#[test]
fn two_writers_racing_version_zero_commit_exactly_one_version_and_one_event() -> TestResult {
    // Given: a subscriber installed before two writers that both read version 0.
    let root = Root::new()?;
    let core = Arc::new(root.open(StoreOptions::default())?);
    let observer = core.subscribe(None)?;

    // When: both compare-and-swap at the same moment.
    let records = race(&core)?;

    // Then: exactly one commits version 1 and the other retains a typed conflict.
    let states: Vec<&Value> = records.iter().map(|record| at(record, "/state")).collect();
    assert_eq!(
        states.iter().filter(|state| **state == "committed").count(),
        1,
        "{records:?}"
    );
    assert_eq!(
        states.iter().filter(|state| **state == "failed").count(),
        1,
        "{records:?}"
    );
    let winner = records
        .iter()
        .find(|record| at(record, "/state") == "committed")
        .ok_or("winner")?;
    let loser = records
        .iter()
        .find(|record| at(record, "/state") == "failed")
        .ok_or("loser")?;
    assert_eq!(at(winner, "/result/version"), "1");
    assert_eq!(at(loser, "/error/kind"), "conflict");
    let events = drain(&observer.delivery);
    let changed = changes(&events);
    assert_eq!(changed.len(), 1, "exactly one doc.changed: {events:?}");
    let change = changed.first().ok_or("change")?;
    assert_eq!(
        serde_json::to_value(&change.data)?,
        json!({"kind": "doc_changed", "data": {"name": "shared", "version": "1"}})
    );
    assert_eq!(
        at(winner, "/result/seq"),
        &json!(change.cursor.seq.get().to_string())
    );
    let document = get(&core, "shared")?;
    assert_eq!(document.version.get(), 1);
    assert_eq!(&document.value, &winner_value(winner, &records)?);
    Ok(Arc::try_unwrap(core)
        .map_err(|_| "core still shared")?
        .close()?)
}

fn winner_value(winner: &Value, records: &[Value]) -> TestResult<Value> {
    let index = records
        .iter()
        .position(|record| record == winner)
        .ok_or("index")?;
    Ok(json!({"writer": index}))
}

#[test]
fn a_conflicting_write_leaves_version_one_and_a_retry_replays_the_same_failure() -> TestResult {
    // Given: a document committed at version 1.
    let root = Root::new()?;
    let core = root.open(StoreOptions::default())?;
    let first = set(
        &core,
        &op_minted(0)?,
        json!({"name": "shared", "version": "0", "value": {"x": 1}}),
    )??;
    assert_eq!(first.state, OpenEnum::Known(OpState::Committed));
    let observer = core.subscribe(None)?;

    // When: a writer still holding version 0 writes, then retries with the same op id.
    let stale = op_minted(0)?;
    let injected = json!({"name": "shared", "version": "0", "value": {"x": 2}});
    let rejected = set(&core, &stale, injected.clone())??;
    let retried = set(&core, &stale, injected)??;

    // Then: both answers are the one retained conflict; the document and events are unchanged.
    assert_eq!(rejected.state, OpenEnum::Known(OpState::Failed));
    assert_eq!(failed_kind(&rejected).as_deref(), Some("conflict"));
    assert_eq!(
        retried, rejected,
        "a retry must return the retained failure, not write"
    );
    let document = get(&core, "shared")?;
    assert_eq!(
        (document.version.get(), document.value),
        (1, json!({"x": 1}))
    );
    assert_eq!(changes(&drain(&observer.delivery)).len(), 0);
    Ok(core.close()?)
}

#[test]
fn null_deletes_but_keeps_the_version_and_absence_reads_as_null_at_zero() -> TestResult {
    // Given: a never-written name and a written one.
    let root = Root::new()?;
    let core = root.open(StoreOptions::default())?;
    let absent = get(&core, "never")?;
    set(
        &core,
        &op_minted(0)?,
        json!({"name": "gone", "version": "0", "value": [1, 2]}),
    )??;

    // When: the written document is set to null at its version.
    let deleted = set(
        &core,
        &op_minted(0)?,
        json!({"name": "gone", "version": "1", "value": null}),
    )??;

    // Then: absence is null at 0; deletion is null at the next version, which still fences.
    assert_eq!((absent.value, absent.version.get()), (Value::Null, 0));
    assert_eq!(at(&deleted.result.ok_or("result")?, "/version"), "2");
    let gone = get(&core, "gone")?;
    assert_eq!((gone.value, gone.version.get()), (Value::Null, 2));
    let stale = set(
        &core,
        &op_minted(0)?,
        json!({"name": "gone", "version": "0", "value": 3}),
    )??;
    assert_eq!(failed_kind(&stale).as_deref(), Some("conflict"));
    Ok(core.close()?)
}

#[test]
fn an_expired_op_id_cannot_become_a_new_document_write() -> TestResult {
    // Given: an op id minted one millisecond before the 24 h deduplication window.
    let root = Root::new()?;
    let core = root.open(StoreOptions::default())?;
    let expired = op_minted(86_400_001)?;
    let observer = core.subscribe(None)?;

    // When: it is presented as a fresh write.
    let outcome = set(
        &core,
        &expired,
        json!({"name": "late", "version": "0", "value": 1}),
    )?;

    // Then: it is refused as expired and nothing is committed.
    let error = outcome.err().ok_or("an expired op id must not write")?;
    assert_eq!(error.kind(), "op_expired");
    assert!(matches!(
        observer.delivery.try_recv(),
        Err(EventError::TryReceive(std::sync::mpsc::TryRecvError::Empty))
    ));
    assert_eq!(get(&core, "late")?.version.get(), 0);
    Ok(core.close()?)
}

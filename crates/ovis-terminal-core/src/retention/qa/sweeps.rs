//! Sweeps commit floors, tombstones and one event together, under a virtual clock.

use ovis_terminal_protocol::domain::documents_retention::{
    Retained, RetentionClass, RetentionLimit, RetentionPolicy, RetentionPosition, RetentionUnit,
};
use ovis_terminal_protocol::schema::scalar::{EventSeq, TimestampMs};
use ovis_terminal_store::{ResourceKey, StoreLimits, StoreOptions, Version};
use serde_json::{Value, json};

use super::fixture::{Core, NOW_MS, Root, TestResult, at, get, identity, op_minted, set};
use crate::events::{Cursor, MissedReason};
use crate::retention::{Candidate, Sweep, SweepOutcome};

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

fn policy() -> RetentionPolicy {
    RetentionPolicy::v1()
        .with_limit(RetentionLimit::new(
            RetentionClass::OutputHistory,
            RetentionUnit::Bytes,
            10,
        ))
        .with_limit(RetentionLimit::new(
            RetentionClass::Events,
            RetentionUnit::Records,
            1,
        ))
}

fn sweep(class: RetentionClass, records: Vec<Candidate>) -> TestResult<Sweep> {
    let limit = policy().limit(class).ok_or("limit")?.clone();
    let swept = Sweep::new(limit, TimestampMs::new(NOW_MS), identity()?.machine_id());
    Ok(records.into_iter().fold(swept, Sweep::record))
}

/// Records at positions 1..=3, written eight virtual days before now; output chunks weigh
/// ten bytes each.
fn aged_records(class: RetentionClass) -> Vec<Candidate> {
    let weight = if class == RetentionClass::OutputHistory {
        10
    } else {
        1
    };
    (1..=3)
        .map(|position| {
            Candidate::new(
                RetentionPosition::new(position),
                TimestampMs::new(NOW_MS - 8 * DAY_MS),
            )
            .weight(weight)
        })
        .collect()
}

fn aged() -> Vec<Candidate> {
    aged_records(RetentionClass::PluginRuns)
}

fn floors(core: &Core) -> TestResult<Vec<(RetentionClass, u64)>> {
    let floors = core.read_snapshot()?.sections.floors;
    Ok(floors
        .floors
        .iter()
        .map(|floor| (floor.class, floor.floor.get()))
        .collect())
}

#[test]
fn advancing_virtual_expiry_moves_all_five_floors_with_a_tombstone_and_one_event_each() -> TestResult
{
    // Given: a document standing in for a stored plugin run, and a subscriber.
    let root = Root::new()?;
    let core = root.open(StoreOptions::default())?;
    set(
        &core,
        &op_minted(0)?,
        json!({"name": "plugin-run-1", "version": "0", "value": {"exit": 0}}),
    )??;
    let observer = core.subscribe(None)?;
    let stored = ResourceKey::new("document", "plugin-run-1");

    // When: every class is swept eight virtual days after its records were written.
    for class in RetentionClass::ALL {
        let mut records = aged_records(class);
        if class == RetentionClass::PluginRuns
            && let Some(oldest) = records.first_mut()
        {
            *oldest = oldest.clone().stored(stored.clone(), Version::new(1));
        }
        assert!(
            matches!(
                sweep(class, records)?.commit(&core)?,
                SweepOutcome::Advanced { .. }
            ),
            "{class:?}"
        );
    }

    // Then: capacity-bound classes keep their newest record, aged classes keep none, and
    // each advance is one durable event.
    let expected = [
        (RetentionClass::OutputHistory, 3),
        (RetentionClass::Events, 3),
        (RetentionClass::Receipts, 4),
        (RetentionClass::PluginRuns, 4),
        (RetentionClass::FinishedTerminals, 4),
    ];
    assert_eq!(floors(&core)?, expected);
    let events: Vec<Value> = std::iter::from_fn(|| observer.delivery.try_recv().ok())
        .map(|event| serde_json::to_value(event.data))
        .collect::<Result<_, _>>()?;
    let classes: Vec<&Value> = events
        .iter()
        .map(|event| at(event, "/data/class"))
        .collect();
    assert_eq!(
        classes,
        [
            "output_history",
            "events",
            "receipts",
            "plugin_runs",
            "finished_terminals"
        ]
    );
    assert_eq!(
        events.get(3),
        Some(
            &json!({"kind": "floor_advanced", "data": {"class": "plugin_runs", "floor": "4", "evicted": "3"}})
        )
    );
    // And: the evicted plugin run is a tombstone whose version still advances, not an absence.
    let tombstone = get(&core, "plugin-run-1")?;
    assert_eq!((tombstone.value, tombstone.version.get()), (Value::Null, 2));
    Ok(core.close()?)
}

#[test]
fn a_repeat_or_older_sweep_never_moves_a_floor_back_and_commits_nothing() -> TestResult {
    // Given: a plugin-run floor already at 4.
    let root = Root::new()?;
    let core = root.open(StoreOptions::default())?;
    sweep(RetentionClass::PluginRuns, aged())?.commit(&core)?;
    let observer = core.subscribe(None)?;

    // When: the same records, then only the oldest one, are swept again.
    let again = sweep(RetentionClass::PluginRuns, aged())?.commit(&core)?;
    let older = sweep(
        RetentionClass::PluginRuns,
        aged().into_iter().take(1).collect(),
    )?
    .commit(&core)?;

    // Then: both are unchanged at floor 4 and no event is published.
    let unchanged = SweepOutcome::Unchanged {
        floor: RetentionPosition::new(4),
    };
    assert_eq!((again, older), (unchanged, unchanged));
    assert!(observer.delivery.try_recv().is_err());
    Ok(core.close()?)
}

#[test]
fn a_stale_tombstone_version_rolls_the_whole_sweep_back() -> TestResult {
    // Given: a stored record at version 1, named with a stale version 7.
    let root = Root::new()?;
    let core = root.open(StoreOptions::default())?;
    set(
        &core,
        &op_minted(0)?,
        json!({"name": "run", "version": "0", "value": 1}),
    )??;
    let records = aged()
        .into_iter()
        .map(|record| record.stored(ResourceKey::new("document", "run"), Version::new(7)))
        .collect();

    // When: the sweep commits.
    let error = sweep(RetentionClass::PluginRuns, records)?
        .commit(&core)
        .err()
        .ok_or("stale CAS must fail")?;

    // Then: a typed conflict, no floor movement and an untouched record.
    assert_eq!(error.kind(), "conflict");
    assert!(floors(&core)?.contains(&(RetentionClass::PluginRuns, 0)));
    assert_eq!(get(&core, "run")?.version.get(), 1);
    Ok(core.close()?)
}

#[test]
fn a_pinned_snapshot_keeps_its_floors_while_a_later_sweep_compacts() -> TestResult {
    // Given: a reader pinned before any eviction.
    let root = Root::new()?;
    let core = root.open(StoreOptions::default())?;
    let pinned = core.subscribe(None)?.snapshot.ok_or("snapshot")?;

    // When: the finished-terminal class is swept.
    sweep(RetentionClass::FinishedTerminals, aged())?.commit(&core)?;

    // Then: the pinned view still reports floor 0, the new view 4, and reads say why.
    let class = RetentionClass::FinishedTerminals;
    assert_eq!(
        pinned
            .sections
            .floors
            .floor(class)
            .map(|floor| floor.floor.get()),
        Some(0)
    );
    let current = core
        .read_snapshot()?
        .sections
        .floors
        .floor(class)
        .ok_or("floor")?;
    assert_eq!(current.floor.get(), 4);
    assert_eq!(
        current.lookup(RetentionPosition::new(2)),
        Retained::Expired {
            floor: current.floor
        }
    );
    assert_eq!(
        current.lookup(RetentionPosition::new(4)),
        Retained::Available
    );
    assert_eq!(
        current.resume_after(RetentionPosition::new(1)),
        Retained::Gap {
            floor: current.floor
        }
    );
    assert_eq!(
        current.resume_after(RetentionPosition::new(3)),
        Retained::Available
    );
    Ok(core.close()?)
}

#[test]
fn store_trim_floors_surface_for_events_and_receipts_and_old_cursors_miss() -> TestResult {
    // Given: a store retaining only four events and four receipts.
    let root = Root::new()?;
    let limits = StoreLimits::default()
        .with_max_events(4)
        .with_max_receipts(4);
    let core = root.open(StoreOptions::default().with_limits(limits))?;

    // When: enough document writes commit to trim both.
    for version in 0..3 {
        set(
            &core,
            &op_minted(0)?,
            json!({"name": "busy", "version": version.to_string(), "value": version}),
        )??;
    }

    // Then: both floors are visible and a cursor below the event floor is an explicit miss.
    let observed = floors(&core)?;
    let floor_of = |class| {
        observed
            .iter()
            .find(|(c, _)| *c == class)
            .map_or(0, |(_, f)| *f)
    };
    assert!(floor_of(RetentionClass::Events) > 1, "{observed:?}");
    assert!(floor_of(RetentionClass::Receipts) > 1, "{observed:?}");
    let identity = identity()?;
    let stale = Cursor::new(
        identity.machine_id(),
        identity.runtime_id(),
        EventSeq::new(1),
    );
    assert_eq!(
        core.subscribe(Some(stale))?.missed,
        Some(MissedReason::CursorExpired)
    );
    Ok(core.close()?)
}

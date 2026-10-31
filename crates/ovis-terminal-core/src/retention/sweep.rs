//! Planning an eviction from a limit and a clock reading, then committing it as one unit.

use ovis_terminal_protocol::domain::documents_retention::{
    RetentionClass, RetentionLimit, RetentionPosition,
};
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_store::{CasWrite, OpId, Payload, ResourceKey, Version};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::floors::{FloorRecord, key, recorded};
use super::{FloorAdvanced, RetentionError};
use crate::events::{
    Commit, Coordinator, Cursor, EventData, EventError, ResourceId, SnapshotProducer,
};
use crate::registry::MachineId;

/// One retained record: its position, when it was recorded, its weight in the limit's unit
/// and the stored resource its eviction tombstones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    position: RetentionPosition,
    recorded_at: TimestampMs,
    weight: u64,
    stored: Option<(ResourceKey, Version)>,
}

impl Candidate {
    /// A record of weight one with no stored resource.
    pub const fn new(position: RetentionPosition, recorded_at: TimestampMs) -> Self {
        Self {
            position,
            recorded_at,
            weight: 1,
            stored: None,
        }
    }

    /// Counts `weight` units, such as the bytes of an output chunk.
    #[must_use]
    pub const fn weight(mut self, weight: u64) -> Self {
        self.weight = weight;
        self
    }

    /// Tombstones `key` at `version` when the record is evicted.
    #[must_use]
    pub fn stored(mut self, key: ResourceKey, version: Version) -> Self {
        self.stored = Some((key, version));
        self
    }

    /// The record's position.
    pub const fn position(&self) -> RetentionPosition {
        self.position
    }
}

/// What a limit evicts and the floor it leaves.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Eviction {
    /// Oldest position still retained after the eviction.
    pub floor: RetentionPosition,
    /// Evicted records, oldest first.
    pub evicted: Vec<Candidate>,
}

/// Evicts the oldest records the limit no longer covers at `now`.
///
/// Positions below the floor are always evicted together, so the prefix ends at the newest
/// position older than the age bound and then grows only as far as the capacity requires.
/// The floor is the oldest survivor's position, or one past the newest evicted record when
/// nothing survives.
///
/// # Errors
///
/// Returns `counter_overflow` when the floor would pass the largest position.
pub fn plan(
    limit: &RetentionLimit,
    now: TimestampMs,
    mut records: Vec<Candidate>,
) -> Result<Eviction, RetentionError> {
    records.sort_by_key(|record| record.position);
    let oldest_allowed = limit
        .max_age_ms
        .map(|age| now.get().saturating_sub(age.get()));
    let aged = records
        .iter()
        .rposition(|record| oldest_allowed.is_some_and(|oldest| record.recorded_at.get() < oldest))
        .map_or(0, |last| last.saturating_add(1));
    let mut retained = records
        .iter()
        .skip(aged)
        .map(|record| record.weight)
        .fold(0, u64::saturating_add);
    let over_capacity = records
        .iter()
        .skip(aged)
        .take_while(|record| {
            let over = retained > limit.capacity.get();
            if over {
                retained = retained.saturating_sub(record.weight);
            }
            over
        })
        .count();
    let survivors = records.split_off(aged.saturating_add(over_capacity));
    let floor = match (survivors.first(), records.last()) {
        (Some(oldest), _) => oldest.position,
        (None, Some(newest)) => RetentionPosition::new(
            newest
                .position
                .get()
                .checked_add(1)
                .ok_or(RetentionError::Exhausted)?,
        ),
        (None, None) => RetentionPosition::new(0),
    };
    Ok(Eviction {
        floor,
        evicted: records,
    })
}

/// A sweep of one class at one clock reading over its currently retained records.
#[derive(Debug, Clone)]
pub struct Sweep {
    limit: RetentionLimit,
    now: TimestampMs,
    machine: MachineId,
    records: Vec<Candidate>,
}

/// Whether a sweep committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SweepOutcome {
    /// The floor advanced; the cursor is the `retention.floor_advanced` event.
    Advanced {
        /// New floor.
        floor: RetentionPosition,
        /// Committed event.
        cursor: Cursor,
    },
    /// Nothing new was evicted and nothing was committed.
    Unchanged {
        /// Current floor.
        floor: RetentionPosition,
    },
}

enum Halt {
    Failed(RetentionError),
    Unchanged(RetentionPosition),
}

impl From<EventError> for Halt {
    fn from(error: EventError) -> Self {
        Self::Failed(error.into())
    }
}

fn halt(error: impl Into<RetentionError>) -> Halt {
    Halt::Failed(error.into())
}

impl Sweep {
    /// Sweeps the class of `limit` at `now` on `machine`'s runtime.
    pub const fn new(limit: RetentionLimit, now: TimestampMs, machine: MachineId) -> Self {
        Self {
            limit,
            now,
            machine,
            records: Vec::new(),
        }
    }

    /// Adds a retained record.
    #[must_use]
    pub fn record(mut self, record: Candidate) -> Self {
        self.records.push(record);
        self
    }

    const fn class(&self) -> RetentionClass {
        self.limit.class
    }

    /// Commits the floor record, a tombstone per evicted record and one floor event in one
    /// transaction. A floor never moves backwards; a stale record version rolls back all of it.
    ///
    /// # Errors
    ///
    /// Returns a typed coordinator, store, record or counter failure; nothing is evicted then.
    pub fn commit<D, P>(self, core: &Coordinator<D, P>) -> Result<SweepOutcome, RetentionError>
    where
        D: EventData + From<FloorAdvanced> + Clone + Serialize + DeserializeOwned,
        P: SnapshotProducer,
    {
        let class = self.class();
        let eviction = plan(&self.limit, self.now, self.records)?;
        let floor = eviction.floor;
        let committed = core.commit_unit(|snapshot| {
            let (current, version) = recorded(snapshot, class).map_err(halt)?;
            if eviction.floor <= current {
                return Err(Halt::Unchanged(current));
            }
            let record = serde_json::to_vec(&FloorRecord {
                floor: eviction.floor,
            })
            .map_err(halt)?;
            let evicted = u64::try_from(eviction.evicted.len())
                .map_err(|_| halt(RetentionError::Exhausted))?;
            let advanced = FloorAdvanced::new(class, eviction.floor, evicted);
            let mut commit = Commit::new(
                OpId::new(uuid::Uuid::now_v7().to_string()),
                D::from(advanced),
                vec![ResourceId::Machine(self.machine)],
            )
            .write(CasWrite::new(
                key(class),
                version,
                Some(Payload::from(record)),
            ));
            for (stored, version) in eviction
                .evicted
                .into_iter()
                .filter_map(|record| record.stored)
            {
                commit = commit.write(CasWrite::new(stored, version, None));
            }
            Ok(commit)
        });
        match committed {
            Ok(cursor) => Ok(SweepOutcome::Advanced { floor, cursor }),
            Err(Halt::Unchanged(floor)) => Ok(SweepOutcome::Unchanged { floor }),
            Err(Halt::Failed(error)) => Err(error),
        }
    }
}

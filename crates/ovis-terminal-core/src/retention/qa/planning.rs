//! The eviction plan under arbitrary records, limits and clock readings.

use ovis_terminal_protocol::domain::documents_retention::{
    RetentionClass, RetentionLimit, RetentionPosition, RetentionUnit,
};
use ovis_terminal_protocol::schema::scalar::{DurationMs, TimestampMs};
use proptest::prelude::*;

use crate::retention::{Candidate, plan};

const NOW: u64 = 1_000_000;

fn records(spec: &[(u64, u64, u64)]) -> Vec<Candidate> {
    spec.iter()
        .map(|&(position, age, weight)| {
            Candidate::new(
                RetentionPosition::new(position),
                TimestampMs::new(NOW.saturating_sub(age)),
            )
            .weight(weight)
        })
        .collect()
}

proptest! {
    #[test]
    fn eviction_is_the_minimal_oldest_prefix_and_the_floor_bounds_it(
        spec in proptest::collection::btree_map(0_u64..1000, (0_u64..2000, 1_u64..8), 0..24),
        capacity in 0_u64..64,
        max_age in proptest::option::of(0_u64..2000),
    ) {
        // Given: records at unique positions with ages and weights, and a limit.
        let spec: Vec<(u64, u64, u64)> = spec.into_iter().map(|(p, (a, w))| (p, a, w)).collect();
        let mut limit = RetentionLimit::new(RetentionClass::PluginRuns, RetentionUnit::Records, capacity);
        if let Some(age) = max_age {
            limit = limit.max_age(DurationMs::new(age));
        }

        // When: the eviction is planned at NOW.
        let eviction = plan(&limit, TimestampMs::new(NOW), records(&spec))
            .map_err(|error| TestCaseError::fail(error.to_string()))?;

        // Then: evicted records are an oldest prefix below the floor, and survivors fit.
        let evicted = eviction.evicted.len();
        let (gone, kept) = spec.split_at(evicted);
        prop_assert!(gone.iter().all(|&(position, _, _)| position < eviction.floor.get()));
        prop_assert!(kept.iter().all(|&(position, _, _)| position >= eviction.floor.get()));
        let weight: u64 = kept.iter().map(|&(_, _, weight)| weight).sum();
        prop_assert!(weight <= capacity, "survivors weigh {} over {}", weight, capacity);
        prop_assert!(kept.iter().all(|&(_, age, _)| max_age.is_none_or(|bound| age <= bound)));
        // And: nothing beyond the newest aged record is evicted unless capacity requires it.
        let aged = spec.iter().rposition(|&(_, age, _)| max_age.is_some_and(|bound| age > bound)).map_or(0, |last| last.saturating_add(1));
        prop_assert!(evicted >= aged);
        if evicted > aged {
            let from_last: u64 = spec.iter().skip(evicted.saturating_sub(1)).map(|&(_, _, weight)| weight).sum();
            prop_assert!(from_last > capacity, "evicted more than the capacity required");
        }
        let expected = kept.first().map_or_else(|| gone.last().map_or(0, |&(position, _, _)| position.saturating_add(1)), |&(position, _, _)| position);
        prop_assert_eq!(eviction.floor.get(), expected);
    }
}

#[test]
fn a_bytes_limit_counts_weights_and_evicting_everything_moves_past_the_newest() -> Result<(), String>
{
    // Given: three 10-byte output chunks at offsets 0, 10, 20 and a 15-byte bound.
    let limit = RetentionLimit::new(RetentionClass::OutputHistory, RetentionUnit::Bytes, 15);
    let chunks = records(&[(0, 0, 10), (10, 0, 10), (20, 0, 10)]);

    // When: planned, and planned again with a zero bound.
    let kept_one =
        plan(&limit, TimestampMs::new(NOW), chunks.clone()).map_err(|e| e.to_string())?;
    let none = RetentionLimit::new(RetentionClass::OutputHistory, RetentionUnit::Bytes, 0);
    let emptied = plan(&none, TimestampMs::new(NOW), chunks).map_err(|e| e.to_string())?;

    // Then: the floor is the oldest surviving offset, or one past the newest evicted one.
    assert_eq!((kept_one.floor.get(), kept_one.evicted.len()), (20, 2));
    assert_eq!((emptied.floor.get(), emptied.evicted.len()), (21, 3));
    Ok(())
}

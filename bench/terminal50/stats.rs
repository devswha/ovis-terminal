//! Latency distributions over every recorded sample; nothing is trimmed or filtered.

use std::time::Duration;

use serde::Serialize;

/// Nearest-rank summary of one sample set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub(super) struct Distribution {
    pub(super) count: usize,
    pub(super) p50_ms: f64,
    pub(super) p95_ms: f64,
    pub(super) p99_ms: f64,
    pub(super) max_ms: f64,
}

/// Milliseconds with microsecond resolution.
pub(super) fn ms(sample: Duration) -> f64 {
    sample.as_secs_f64() * 1e3
}

/// The sample of nearest rank `per_mille / 1000` of the sorted `sorted`.
fn rank(sorted: &[Duration], per_mille: usize) -> Duration {
    let wanted = sorted.len().saturating_mul(per_mille).div_ceil(1000);
    sorted
        .get(wanted.saturating_sub(1))
        .copied()
        .unwrap_or_default()
}

/// Summarizes every sample of `samples`.
pub(super) fn summarize(samples: &[Duration]) -> Distribution {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    Distribution {
        count: sorted.len(),
        p50_ms: ms(rank(&sorted, 500)),
        p95_ms: ms(rank(&sorted, 950)),
        p99_ms: ms(rank(&sorted, 990)),
        max_ms: ms(sorted.last().copied().unwrap_or_default()),
    }
}

/// Samples strictly above `bound`.
pub(super) fn over(samples: &[Duration], bound: Duration) -> usize {
    samples.iter().filter(|sample| **sample > bound).count()
}

/// Every sample in whole microseconds, in recording order, for the raw companion file.
pub(super) fn micros(samples: &[Duration]) -> Vec<u64> {
    samples
        .iter()
        .map(|sample| u64::try_from(sample.as_micros()).unwrap_or(u64::MAX))
        .collect()
}

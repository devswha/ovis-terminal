//! Bounded latency histogram: fixed memory however long a run lasts.

use std::time::Duration;

use ovis_terminal_probes::platform::{Micros, SampleCount};
use serde::Serialize;

/// Width of one bucket.
const BUCKET_MICROS: u64 = 10;
/// Buckets cover 0..1 s; slower samples land in the overflow bucket.
const BUCKETS: usize = 100_000;

/// Latency samples counted in 10-microsecond buckets up to one second, with
/// the exact maximum kept separately.
#[derive(Clone, Debug)]
pub struct LatencyHistogram {
    buckets: Vec<u64>,
    overflow: u64,
    count: u64,
    max: Micros,
}

/// Percentiles of a histogram; each is the upper edge of its 10-microsecond
/// bucket, capped at the exact maximum.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct LatencySummary {
    /// Number of samples.
    pub samples: SampleCount,
    /// Median.
    pub p50: Micros,
    /// 95th percentile.
    pub p95: Micros,
    /// 99th percentile.
    pub p99: Micros,
    /// Exact maximum.
    pub max: Micros,
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self {
            buckets: vec![0; BUCKETS],
            overflow: 0,
            count: 0,
            max: Micros::default(),
        }
    }
}

impl LatencyHistogram {
    /// Counts one sample.
    pub fn record(&mut self, latency: Duration) {
        let micros = Micros::from_duration(latency);
        self.max = self.max.max(micros);
        self.count = self.count.saturating_add(1);
        let slot = usize::try_from(micros.get() / BUCKET_MICROS)
            .ok()
            .and_then(|index| self.buckets.get_mut(index));
        match slot {
            Some(bucket) => *bucket = bucket.saturating_add(1),
            None => self.overflow = self.overflow.saturating_add(1),
        }
    }

    /// Adds every sample of `other`.
    pub fn merge(&mut self, other: &Self) {
        for (mine, theirs) in self.buckets.iter_mut().zip(&other.buckets) {
            *mine = mine.saturating_add(*theirs);
        }
        self.overflow = self.overflow.saturating_add(other.overflow);
        self.count = self.count.saturating_add(other.count);
        self.max = self.max.max(other.max);
    }

    /// Nearest-rank percentiles.
    #[must_use]
    pub fn summary(&self) -> LatencySummary {
        LatencySummary {
            samples: SampleCount::new(self.count),
            p50: self.percentile(50),
            p95: self.percentile(95),
            p99: self.percentile(99),
            max: self.max,
        }
    }

    fn percentile(&self, percent: u64) -> Micros {
        let rank = self.count.saturating_mul(percent).div_ceil(100).max(1);
        let mut seen = 0_u64;
        for (index, bucket) in self.buckets.iter().enumerate() {
            seen = seen.saturating_add(*bucket);
            if seen >= rank {
                let upper = u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1);
                return Micros::new(upper.saturating_mul(BUCKET_MICROS).min(self.max.get()));
            }
        }
        self.max
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::LatencyHistogram;

    #[test]
    fn percentiles_follow_nearest_rank_and_max_stays_exact() {
        let mut histogram = LatencyHistogram::default();
        for micros in 1..=100 {
            histogram.record(Duration::from_micros(micros * 100));
        }
        histogram.record(Duration::from_secs(3));
        let summary = histogram.summary();
        assert_eq!(summary.samples.get(), 101);
        assert_eq!(summary.p50.get(), 5_110);
        assert_eq!(summary.p99.get(), 10_010);
        assert_eq!(summary.max.get(), 3_000_000);
    }

    #[test]
    fn merging_equals_recording_into_one() {
        let (mut left, mut right, mut both) = (
            LatencyHistogram::default(),
            LatencyHistogram::default(),
            LatencyHistogram::default(),
        );
        for micros in [5_u64, 70, 900, 12_000, 2_000_000] {
            let sample = Duration::from_micros(micros);
            if micros % 2 == 0 {
                left.record(sample);
            } else {
                right.record(sample);
            }
            both.record(sample);
        }
        left.merge(&right);
        assert_eq!(left.summary(), both.summary());
    }
}

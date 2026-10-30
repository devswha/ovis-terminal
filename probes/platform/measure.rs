//! Measured quantities carried by probe and harness reports.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A count of bytes: output moved through a terminal or resident memory.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct ByteCount(#[serde(with = "super::decimal")] u64);

impl ByteCount {
    /// Wraps a byte count.
    #[must_use]
    pub const fn new(bytes: u64) -> Self {
        Self(bytes)
    }

    /// The byte count.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Adds `bytes`, saturating at `u64::MAX` rather than wrapping.
    #[must_use]
    pub const fn saturating_add(self, bytes: u64) -> Self {
        Self(self.0.saturating_add(bytes))
    }
}

/// A number of samples, rounds or other counted events.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct SampleCount(#[serde(with = "super::decimal")] u64);

impl SampleCount {
    /// Wraps a count.
    #[must_use]
    pub const fn new(count: u64) -> Self {
        Self(count)
    }

    /// The count.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A measured duration in whole microseconds.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct Micros(#[serde(with = "super::decimal")] u64);

impl Micros {
    /// Wraps a microsecond count.
    #[must_use]
    pub const fn new(micros: u64) -> Self {
        Self(micros)
    }

    /// Converts a duration, saturating at `u64::MAX` microseconds (about
    /// 584,000 years) instead of failing.
    #[must_use]
    pub fn from_duration(duration: Duration) -> Self {
        Self(u64::try_from(duration.as_micros()).unwrap_or(u64::MAX))
    }

    /// The microsecond count.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The same span as a [`Duration`].
    #[must_use]
    pub const fn as_duration(self) -> Duration {
        Duration::from_micros(self.0)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::{any, proptest};

    use super::{ByteCount, Micros};

    #[test]
    fn u64_max_round_trips_exactly_as_a_string() {
        let json = serde_json::to_string(&ByteCount::new(u64::MAX)).unwrap();
        assert_eq!(json, "\"18446744073709551615\"");
        assert_eq!(
            serde_json::from_str::<ByteCount>(&json).unwrap().get(),
            u64::MAX
        );
    }

    #[test]
    fn numeric_and_non_canonical_forms_are_rejected() {
        for rejected in [
            "7",
            "\"07\"",
            "\"+7\"",
            "\"-7\"",
            "\"\"",
            "\" 7\"",
            "\"18446744073709551616\"",
        ] {
            assert!(
                serde_json::from_str::<Micros>(rejected).is_err(),
                "{rejected} was accepted"
            );
        }
    }

    proptest! {
        #[test]
        fn every_u64_round_trips(value in any::<u64>()) {
            let json = serde_json::to_string(&Micros::new(value)).unwrap();
            assert_eq!(json, format!("\"{value}\""));
            assert_eq!(serde_json::from_str::<Micros>(&json).unwrap(), Micros::new(value));
        }
    }
}

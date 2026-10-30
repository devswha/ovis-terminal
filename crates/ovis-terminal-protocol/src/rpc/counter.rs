//! Canonical decimal-string encoding for 64-bit protocol counters.

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A 64-bit counter, sequence, revision, offset or generation carried as a
/// canonical decimal JSON string so JavaScript clients cannot lose precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DecimalU64(u64);

impl DecimalU64 {
    /// Wraps a native counter value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the native counter value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Serialize for DecimalU64 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

struct DecimalVisitor;

impl Visitor<'_> for DecimalVisitor {
    type Value = DecimalU64;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a canonical decimal string holding an unsigned 64-bit integer")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        let canonical = value.len() == 1 || !value.starts_with('0');
        if !canonical || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(E::invalid_value(de::Unexpected::Str(value), &self));
        }
        value
            .parse::<u64>()
            .map(DecimalU64)
            .map_err(|_| E::invalid_value(de::Unexpected::Str(value), &self))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Err(E::invalid_type(de::Unexpected::Unsigned(value), &self))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Err(E::invalid_type(de::Unexpected::Signed(value), &self))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
        Err(E::invalid_type(de::Unexpected::Float(value), &self))
    }
}

impl<'de> Deserialize<'de> for DecimalU64 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(DecimalVisitor)
    }
}

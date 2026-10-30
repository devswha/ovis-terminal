//! Canonical decimal-string encoding for u64 values in probe records.
//!
//! JavaScript clients lose precision above 2^53, so every u64 travels as a
//! string without sign, padding or leading zeros; a JSON number is rejected.

use serde::de::{Error, Unexpected, Visitor};
use serde::{Deserializer, Serializer};

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `with` attribute fixes the `&u64` signature"
)]
pub(super) fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(value)
}

pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    deserializer.deserialize_str(CanonicalDecimal)
}

struct CanonicalDecimal;

impl Visitor<'_> for CanonicalDecimal {
    type Value = u64;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a canonical decimal string holding a u64")
    }

    fn visit_str<E: Error>(self, text: &str) -> Result<u64, E> {
        let canonical = text == "0"
            || (text.bytes().all(|byte| byte.is_ascii_digit()) && !text.starts_with('0'));
        if !canonical {
            return Err(E::invalid_value(Unexpected::Str(text), &self));
        }
        text.parse()
            .map_err(|_overflow| E::invalid_value(Unexpected::Str(text), &self))
    }
}

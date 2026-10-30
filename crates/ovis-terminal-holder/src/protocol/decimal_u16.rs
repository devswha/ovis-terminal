//! Canonical decimal-string serde for JSON u16 fields.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serializer};

/// Serializes a u16 as a canonical JSON decimal string.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde with serializers require a reference to the serialized field"
)]
pub(super) fn serialize<S>(value: &u16, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

/// Deserializes only canonical non-negative decimal strings.
pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let parsed = value.parse::<u16>().map_err(D::Error::custom)?;
    if parsed.to_string() != value {
        return Err(D::Error::custom("non-canonical u16 decimal string"));
    }
    Ok(parsed)
}

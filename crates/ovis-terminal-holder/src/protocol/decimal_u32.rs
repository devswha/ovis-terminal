//! Canonical decimal-string serde for JSON u32 fields.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serializer};

/// Serializes a u32 as a canonical JSON decimal string.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde with serializers require a reference to the serialized field"
)]
pub(super) fn serialize<S>(value: &u32, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

/// Deserializes only canonical non-negative decimal strings.
pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let parsed = value.parse::<u32>().map_err(D::Error::custom)?;
    if parsed.to_string() != value {
        return Err(D::Error::custom("non-canonical u32 decimal string"));
    }
    Ok(parsed)
}

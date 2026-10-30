//! Canonical decimal-string serde for optional JSON u32 fields.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serializer};

/// Serializes an optional u32 as null or a canonical decimal string.
#[expect(
    clippy::ref_option,
    reason = "serde with serializers require a reference to the serialized field"
)]
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde with serializers require a reference to the serialized field"
)]
pub(super) fn serialize<S>(value: &Option<u32>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(value) => serializer.serialize_some(&value.to_string()),
        None => serializer.serialize_none(),
    }
}

/// Deserializes null or a canonical non-negative decimal string.
pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    value
        .map(|raw| {
            let parsed = raw.parse::<u32>().map_err(D::Error::custom)?;
            if parsed.to_string() != raw {
                return Err(D::Error::custom("non-canonical u32 decimal string"));
            }
            Ok(parsed)
        })
        .transpose()
}

//! Canonical decimal-string encoding for u64 counters in JSON.
//!
//! JSON numbers lose precision above 2^53 in common clients, so every u64
//! offset and revision travels as a string such as `"3"`. A JSON number, a
//! sign, a leading zero or any non-digit is rejected at decode.

use serde::Serializer;
use serde::de::{Deserialize, Deserializer, Error};

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde `with` fixes the `&T` signature"
)]
pub(super) fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(value)
}

pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let text = String::deserialize(deserializer)?;
    let canonical = !text.is_empty()
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    if !canonical {
        return Err(D::Error::custom(format!(
            "{text:?} is not a canonical decimal u64"
        )));
    }
    text.parse()
        .map_err(|_| D::Error::custom(format!("{text:?} exceeds u64")))
}

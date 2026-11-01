//! Signed clock skew carried as a canonical decimal string.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::schema::scalar::TimestampMs;

/// Text that is not a canonical signed decimal integer.
#[derive(Debug, Error)]
#[error("expected a canonical signed decimal integer")]
#[non_exhaustive]
pub struct SkewError;

impl SkewError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        "invalid_params"
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        "send milliseconds as a decimal string without '+' or leading zeros"
    }
}

/// Milliseconds a remote clock is ahead (positive) or behind (negative) the local one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SkewMs(i64);

impl SkewMs {
    /// Wraps a signed millisecond count.
    #[must_use]
    pub const fn new(millis: i64) -> Self {
        Self(millis)
    }

    /// The signed millisecond count.
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }

    /// `remote - local`, saturating at the bounds of `i64`.
    #[must_use]
    pub fn between(remote: TimestampMs, local: TimestampMs) -> Self {
        let (remote, local) = (i128::from(remote.get()), i128::from(local.get()));
        let skew = remote.saturating_sub(local);
        Self(i64::try_from(skew).unwrap_or(if skew < 0 { i64::MIN } else { i64::MAX }))
    }
}

impl TryFrom<String> for SkewMs {
    type Error = SkewError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let value: i64 = text.parse().map_err(|_| SkewError)?;
        if value.to_string() != text {
            return Err(SkewError);
        }
        Ok(Self(value))
    }
}

impl From<SkewMs> for String {
    fn from(skew: SkewMs) -> Self {
        skew.0.to_string()
    }
}

impl JsonSchema for SkewMs {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("SkewMs")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^(0|-?[1-9][0-9]*)$",
            "description": "Signed milliseconds carried as a canonical decimal string.",
        })
    }
}

//! The advertised bound a `limit_exceeded` refusal reached.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::schema::scalar::Count;

/// Name of a limit advertised in `hello` limits.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct LimitName(String);

impl LimitName {
    /// Wraps the `hello` limits field name.
    #[must_use]
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Borrows the field name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The advertised limit a refusal reached and its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ExceededBound {
    /// The `hello` limits field that was reached.
    pub limit: LimitName,
    /// Its advertised value.
    pub value: Count,
}

impl ExceededBound {
    /// Names an advertised limit and its value.
    pub const fn new(limit: LimitName, value: Count) -> Self {
        Self { limit, value }
    }
}

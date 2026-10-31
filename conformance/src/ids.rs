//! Identities the runner keeps distinct: scenarios, cases, connections and counters.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::RunnerError;

/// `family/name`, for example `protocol/multiplex`; also the file `family/name.json`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct ScenarioId(String);

impl ScenarioId {
    /// Parses two lowercase segments; the family may start with `_` (runner-internal).
    pub(crate) fn parse(text: &str) -> Result<Self, RunnerError> {
        let segment = |part: &str, leading: bool| {
            let body = if leading {
                part.strip_prefix('_').unwrap_or(part)
            } else {
                part
            };
            !body.is_empty()
                && body
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        };
        match text.split_once('/') {
            Some((family, name)) if segment(family, true) && segment(name, false) => {
                Ok(Self(text.to_owned()))
            }
            _ => Err(RunnerError::ScenarioMissing {
                scenario: text.to_owned(),
            }),
        }
    }

    /// Path of the scenario file below `conformance/scenarios/`.
    pub(crate) fn relative_path(&self) -> PathBuf {
        PathBuf::from(format!("{}.json", self.0))
    }
}

impl TryFrom<String> for ScenarioId {
    type Error = RunnerError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ScenarioId> for String {
    fn from(value: ScenarioId) -> Self {
        value.0
    }
}

impl fmt::Display for ScenarioId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A case every scenario may declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CaseName {
    /// The contract holds on valid input.
    Happy,
    /// Invalid or hostile input gets the typed rejection.
    Failure,
}

impl CaseName {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Happy => "happy",
            Self::Failure => "failure",
        }
    }
}

/// A scenario-local connection name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct ConnName(String);

impl ConnName {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A u64 that travels as a canonical decimal string, never a JSON number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) struct Decimal(pub(crate) u64);

impl Decimal {
    /// Accepts only `"0"` or digits without a leading zero that fit u64.
    pub(crate) fn parse(text: &str) -> Result<Self, RunnerError> {
        let canonical = text == "0"
            || (!text.starts_with('0')
                && !text.is_empty()
                && text.bytes().all(|b| b.is_ascii_digit()));
        if !canonical {
            return Err(RunnerError::Decimal);
        }
        text.parse().map(Self).map_err(|_| RunnerError::Decimal)
    }

    pub(crate) fn from_usize(value: usize) -> Self {
        Self(u64::try_from(value).unwrap_or(u64::MAX))
    }
}

impl Serialize for Decimal {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::{Decimal, ScenarioId};

    #[test]
    fn counters_round_trip_exactly_at_u64_max_and_reject_numbers() {
        let text = serde_json::to_string(&Decimal(u64::MAX)).ok();
        assert_eq!(text.as_deref(), Some("\"18446744073709551615\""));
        let back: Result<Decimal, _> = serde_json::from_str("\"18446744073709551615\"");
        assert_eq!(back.ok(), Some(Decimal(u64::MAX)));
        for rejected in [
            "0",
            "\"01\"",
            "\"-1\"",
            "\"18446744073709551616\"",
            "\"\"",
            "\"1e3\"",
        ] {
            assert!(
                serde_json::from_str::<Decimal>(rejected).is_err(),
                "{rejected}"
            );
        }
    }

    #[test]
    fn scenario_ids_are_two_lowercase_segments() {
        for accepted in [
            "protocol/multiplex",
            "_runner/self-test",
            "part-a/task-21",
            "all/all",
        ] {
            assert!(ScenarioId::parse(accepted).is_ok(), "{accepted}");
        }
        for rejected in [
            "protocol",
            "../etc/passwd",
            "a/b/c",
            "Protocol/x",
            "a/_b",
            "/x",
            "x/",
        ] {
            assert!(ScenarioId::parse(rejected).is_err(), "{rejected}");
        }
    }
}

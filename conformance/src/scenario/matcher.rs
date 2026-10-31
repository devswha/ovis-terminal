//! Expectations: a JSON pointer per checked value, each one recorded as an assertion.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::labels::Labels;
use crate::ids::Decimal;

/// Pointer to matcher; the empty pointer is the whole document.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(transparent)]
pub(crate) struct Expect(BTreeMap<String, Matcher>);

/// How one pointed-at value is checked.
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "Value")]
pub(crate) enum Matcher {
    /// Equal to this JSON value.
    Exact(Value),
    /// `{"$present": true}`: the value exists (or, with false, is absent).
    Present(bool),
    /// `{"$at_least": "10000"}`: a decimal string not below this bound.
    AtLeast(Decimal),
    /// `{"$contains": [..]}`: an array holding every listed value.
    Contains(Vec<Value>),
    /// `{"$excludes": [..]}`: an array holding none of the listed values.
    Excludes(Vec<Value>),
    /// `{"$equals_file": "schema/x.json"}`: equal to that repository JSON file.
    EqualsFile(String),
}

/// Operator objects are recognised by their single `$` key; any other `$` key is refused.
impl TryFrom<Value> for Matcher {
    type Error = String;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let Value::Object(object) = &value else {
            return Ok(Self::Exact(value));
        };
        let mut keys = object.keys().filter(|key| key.starts_with('$'));
        let Some(operator) = keys.next() else {
            return Ok(Self::Exact(value));
        };
        let (Some(argument), 1) = (object.get(operator), object.len()) else {
            return Err(format!("{operator} must be the only key of its object"));
        };
        let invalid = |what: &str| format!("{operator} takes {what}");
        match (operator.as_str(), argument) {
            ("$present", Value::Bool(flag)) => Ok(Self::Present(*flag)),
            ("$at_least", Value::String(text)) => Decimal::parse(text)
                .map(Self::AtLeast)
                .map_err(|_| invalid("a canonical decimal string")),
            ("$contains", Value::Array(items)) => Ok(Self::Contains(items.clone())),
            ("$excludes", Value::Array(items)) => Ok(Self::Excludes(items.clone())),
            ("$equals_file", Value::String(path)) => Ok(Self::EqualsFile(path.clone())),
            ("$present" | "$at_least" | "$contains" | "$excludes" | "$equals_file", _) => {
                Err(invalid("a different argument type"))
            }
            _ => Err(format!("unknown matcher {operator}")),
        }
    }
}

/// One checked expectation, kept in the evidence whether it held or not.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Assertion {
    pub(crate) check: String,
    pub(crate) expected: Value,
    pub(crate) actual: Value,
    pub(crate) passed: bool,
}

impl Assertion {
    pub(crate) fn equal(check: String, expected: Value, actual: Value) -> Self {
        let passed = expected == actual;
        Self {
            check,
            expected,
            actual,
            passed,
        }
    }
}

/// A checked value and the name it is reported under.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Subject<'a> {
    pub(crate) scope: &'a str,
    pub(crate) value: &'a Value,
}

impl Expect {
    /// Replaces `$op:<label>` strings in literal values with this run's identities.
    pub(crate) fn resolved(&self, labels: &mut Labels) -> Self {
        let entries = self.0.iter().map(|(pointer, matcher)| {
            let matcher = match matcher {
                Matcher::Exact(value) => Matcher::Exact(labels.resolve(value)),
                Matcher::Contains(items) => {
                    Matcher::Contains(items.iter().map(|item| labels.resolve(item)).collect())
                }
                Matcher::Excludes(items) => {
                    Matcher::Excludes(items.iter().map(|item| labels.resolve(item)).collect())
                }
                other => other.clone(),
            };
            (pointer.clone(), matcher)
        });
        Self(entries.collect())
    }

    /// Checks every pointer of `subject`; `repo` resolves `$equals_file`.
    pub(crate) fn check(&self, subject: Subject<'_>, repo: &Path) -> Vec<Assertion> {
        self.0
            .iter()
            .map(|(pointer, matcher)| {
                let check = format!("{}{pointer}", subject.scope);
                matcher.assert(
                    check,
                    Seen {
                        actual: subject.value.pointer(pointer),
                        repo,
                    },
                )
            })
            .collect()
    }
}

/// The pointed-at value, if any, and the repository that `$equals_file` reads from.
#[derive(Debug, Clone, Copy)]
struct Seen<'a> {
    actual: Option<&'a Value>,
    repo: &'a Path,
}

impl Matcher {
    fn assert(&self, check: String, seen: Seen<'_>) -> Assertion {
        let Seen { actual, repo } = seen;
        let shown = actual.cloned().unwrap_or(Value::Null);
        let array = actual.and_then(Value::as_array);
        let (expected, passed) = match self {
            Self::Exact(value) => (value.clone(), actual == Some(value)),
            Self::Present(flag) => (
                serde_json::json!({ "$present": flag }),
                actual.is_some() == *flag,
            ),
            Self::AtLeast(bound) => (
                serde_json::json!({ "$at_least": bound }),
                actual
                    .and_then(Value::as_str)
                    .and_then(|text| Decimal::parse(text).ok())
                    .is_some_and(|value| value >= *bound),
            ),
            Self::Contains(items) => (
                serde_json::json!({ "$contains": items }),
                array.is_some_and(|values| items.iter().all(|item| values.contains(item))),
            ),
            Self::Excludes(items) => (
                serde_json::json!({ "$excludes": items }),
                array.is_some_and(|values| items.iter().all(|item| !values.contains(item))),
            ),
            Self::EqualsFile(path) => {
                let file = std::fs::read(repo.join(path))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
                (
                    serde_json::json!({ "$equals_file": path }),
                    file.is_some() && file.as_ref() == actual,
                )
            }
        };
        Assertion {
            check,
            expected,
            actual: shown,
            passed,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Expect, Subject};

    fn expect(value: serde_json::Value) -> Result<Expect, serde_json::Error> {
        serde_json::from_value(value)
    }

    #[test]
    fn every_operator_holds_and_fails_on_the_values_it_names() -> Result<(), serde_json::Error> {
        let expect = expect(json!({
            "/id": "a", "/missing": {"$present": false}, "/n": {"$at_least": "10"},
            "/list": {"$contains": [1, 2]}, "/other": {"$excludes": ["x"]},
        }))?;
        let good = json!({"id": "a", "n": "10", "list": [2, 1, 3], "other": ["y"]});
        let bad = json!({"id": "b", "missing": 0, "n": 10, "list": [1], "other": ["x"]});
        let subject = |value| Subject { scope: "r", value };
        let root = std::path::Path::new(".");
        assert!(expect.check(subject(&good), root).iter().all(|a| a.passed));
        let failed = expect.check(subject(&bad), root);
        assert_eq!(failed.len(), 5);
        assert!(failed.iter().all(|a| !a.passed), "{failed:?}");
        Ok(())
    }

    #[test]
    fn unknown_or_mixed_operators_are_refused_instead_of_compared_literally() {
        for refused in [
            json!({"/a": {"$presnt": true}}),
            json!({"/a": {"$present": true, "x": 1}}),
            json!({"/a": {"$at_least": 10}}),
        ] {
            assert!(expect(refused.clone()).is_err(), "{refused}");
        }
    }
}

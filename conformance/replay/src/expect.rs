//! Expectations over a reply or event: one matcher per JSON pointer.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bind::Bindings;
use crate::error::ReplayError;

/// Pointer to matcher; the empty pointer is the whole document.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(transparent)]
#[non_exhaustive]
pub struct Expect(BTreeMap<String, Matcher>);

/// How one pointed-at value is checked.
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "Value")]
#[non_exhaustive]
pub enum Matcher {
    /// Equal to this JSON value after binding.
    Exact(Value),
    /// `{"$present": bool}`: the value exists, or is absent.
    Present(bool),
    /// `{"$contains": [..]}`: an array holding every listed value.
    Contains(Vec<Value>),
    /// `{"$excludes": [..]}`: an array holding none of the listed values.
    Excludes(Vec<Value>),
    /// `{"$capture": "name"}`: the value exists; later `$value:name` strings become it.
    Capture(String),
}

/// Operator objects are recognised by their single `$` key; any other `$` key is refused.
impl TryFrom<Value> for Matcher {
    type Error = String;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let Value::Object(object) = &value else {
            return Ok(Self::Exact(value));
        };
        let Some(operator) = object.keys().find(|key| key.starts_with('$')) else {
            return Ok(Self::Exact(value));
        };
        let (Some(argument), 1) = (object.get(operator), object.len()) else {
            return Err(format!("{operator} must be the only key of its object"));
        };
        match (operator.as_str(), argument) {
            ("$present", Value::Bool(flag)) => Ok(Self::Present(*flag)),
            ("$contains", Value::Array(items)) => Ok(Self::Contains(items.clone())),
            ("$excludes", Value::Array(items)) => Ok(Self::Excludes(items.clone())),
            ("$capture", Value::String(name)) => Ok(Self::Capture(name.clone())),
            ("$present" | "$contains" | "$excludes" | "$capture", _) => {
                Err(format!("{operator} takes a different argument type"))
            }
            _ => Err(format!("unknown matcher {operator}")),
        }
    }
}

/// One evaluated expectation, kept in the evidence whether it held or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Check {
    /// `<case>/<step><pointer>`.
    pub check: String,
    /// What was required.
    pub expected: Value,
    /// What was seen; `null` when absent.
    pub actual: Value,
    /// Whether it held.
    pub passed: bool,
}

impl Check {
    /// An equality check.
    #[must_use]
    pub fn equal(check: String, expected: Value, actual: Value) -> Self {
        let passed = expected == actual;
        Self {
            check,
            expected,
            actual,
            passed,
        }
    }

    /// The same check reported under `prefix/`.
    #[must_use]
    pub fn scoped(self, prefix: &str) -> Self {
        Self {
            check: format!("{prefix}/{}", self.check),
            ..self
        }
    }
}

/// An array matcher's bound items, or the unbound symbol it names.
fn items(values: &[Value], bindings: &mut Bindings) -> Result<Vec<Value>, ReplayError> {
    values.iter().map(|value| bindings.resolve(value)).collect()
}

impl Expect {
    /// Checks every pointer of `subject`, binding literals first and recording captures.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayError::Unbound`] when an expected literal names an unbound symbol.
    pub fn check(
        &self,
        (scope, subject): (&str, &Value),
        bindings: &mut Bindings,
    ) -> Result<Vec<Check>, ReplayError> {
        let mut checks = Vec::with_capacity(self.0.len());
        for (pointer, matcher) in &self.0 {
            let actual = subject.pointer(pointer);
            let array = actual.and_then(Value::as_array);
            let (expected, passed) = match matcher {
                Matcher::Exact(value) => {
                    let expected = bindings.resolve(value)?;
                    let passed = actual == Some(&expected);
                    (expected, passed)
                }
                Matcher::Present(flag) => (
                    serde_json::json!({ "$present": flag }),
                    actual.is_some() == *flag,
                ),
                Matcher::Contains(listed) => {
                    let wanted = items(listed, bindings)?;
                    let passed =
                        array.is_some_and(|values| wanted.iter().all(|item| values.contains(item)));
                    (serde_json::json!({ "$contains": wanted }), passed)
                }
                Matcher::Excludes(listed) => {
                    let refused = items(listed, bindings)?;
                    let passed = array
                        .is_some_and(|values| refused.iter().all(|item| !values.contains(item)));
                    (serde_json::json!({ "$excludes": refused }), passed)
                }
                Matcher::Capture(name) => {
                    if let Some(value) = actual {
                        bindings.capture(name, value.clone());
                    }
                    (serde_json::json!({ "$capture": name }), actual.is_some())
                }
            };
            checks.push(Check {
                check: format!("{scope}{pointer}"),
                expected,
                actual: actual.cloned().unwrap_or(Value::Null),
                passed,
            });
        }
        Ok(checks)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Expect;
    use crate::bind::Bindings;

    fn expect(value: serde_json::Value) -> Result<Expect, serde_json::Error> {
        serde_json::from_value(value)
    }

    #[test]
    fn every_operator_holds_and_fails_on_the_values_it_names()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut bindings = Bindings::default();
        bindings.capture("ws", json!("workspace:1"));
        let expect = expect(json!({
            "/id": "$value:ws", "/missing": {"$present": false},
            "/list": {"$contains": ["$value:ws"]}, "/other": {"$excludes": ["x"]},
            "/rev": {"$capture": "rev"},
        }))?;
        let good =
            json!({"id": "workspace:1", "list": ["workspace:1"], "other": ["y"], "rev": "4"});
        let bad = json!({"id": "workspace:2", "missing": 0, "list": [], "other": ["x"]});
        assert!(
            expect
                .check(("r", &good), &mut bindings)?
                .iter()
                .all(|c| c.passed)
        );
        assert_eq!(bindings.resolve(&json!("$value:rev"))?, json!("4"));
        let unbound = expect.check(("r", &bad), &mut Bindings::default());
        assert_eq!(unbound.map_err(|e| e.kind()).err(), Some("symbol_unbound"));
        let mut bound = Bindings::default();
        bound.capture("ws", json!("workspace:1"));
        let checks = expect.check(("r", &bad), &mut bound)?;
        assert_eq!(checks.iter().filter(|c| !c.passed).count(), 5, "{checks:?}");
        Ok(())
    }

    #[test]
    fn an_exact_object_matches_only_an_equal_object() -> Result<(), Box<dyn std::error::Error>> {
        let expect = expect(json!({"/data": {"lease": "lease:1", "reason": "disconnected"}}))?;
        let check = |value: serde_json::Value| -> Result<bool, crate::ReplayError> {
            Ok(expect
                .check(("r", &json!({"data": value})), &mut Bindings::default())?
                .iter()
                .all(|c| c.passed))
        };
        assert!(check(
            json!({"reason": "disconnected", "lease": "lease:1"})
        )?);
        for other in [
            json!({"lease": "lease:2", "reason": "disconnected"}),
            json!({"lease": "lease:1"}),
            json!({"lease": "lease:1", "reason": "disconnected", "extra": 1}),
            json!({}),
            json!(null),
        ] {
            assert!(!check(other.clone())?, "{other}");
        }
        Ok(())
    }

    #[test]
    fn unknown_or_mixed_operators_are_refused_instead_of_compared_literally() {
        for refused in [
            json!({"/a": {"$presnt": true}}),
            json!({"/a": {"$present": true, "x": 1}}),
            json!({"/a": {"$capture": 1}}),
        ] {
            assert!(expect(refused.clone()).is_err(), "{refused}");
        }
    }
}

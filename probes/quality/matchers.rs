//! Recomputing a recorded assertion from its expected and actual values, using the
//! conformance runner's matcher operators. An assertion the record cannot settle is never
//! accepted: it is reported instead of being trusted.

use std::path::Path;

use serde_json::Value;

/// What recomputing one recorded assertion established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Judgement {
    /// The assertion holds (`true`) or fails (`false`) on its recorded values.
    Holds(bool),
    /// `expected` uses an operator the runner does not define.
    UnknownMatcher(String),
    /// The operator is known but its operand cannot be read, such as a missing file.
    Unjudgeable(String),
}

fn decimal(value: &Value) -> Option<u128> {
    let text = value.as_str()?;
    (text == "0" || !text.starts_with('0')).then(|| text.parse().ok())?
}

/// `$equals_file`: the repository file, parsed as JSON, equals `actual`.
fn equals_file(operand: &Value, actual: &Value, repo: &Path) -> Judgement {
    let Some(relative) = operand.as_str() else {
        return Judgement::Unjudgeable("$equals_file names no path".to_owned());
    };
    let file = std::fs::read(repo.join(relative))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    file.map_or_else(
        || {
            Judgement::Unjudgeable(format!(
                "{relative} is not a readable JSON file under the repository"
            ))
        },
        |file| Judgement::Holds(&file == actual),
    )
}

/// Recomputes the assertion named `check` from `(expected, actual)`.
///
/// The runner records a missing value as `null`, so `$present: true` and `$capture` hold
/// only for a non-null actual and `$present: false` only for a null one.
/// `expect_failure/kind` compares only the failure kind.
pub(super) fn holds(check: &str, (expected, actual): (&Value, &Value), repo: &Path) -> Judgement {
    if check == "expect_failure/kind" {
        let kind = expected.get("kind");
        return Judgement::Holds(kind.is_some() && kind == actual.get("kind"));
    }
    let operator = expected
        .as_object()
        .filter(|object| object.len() == 1)
        .and_then(|object| object.iter().next())
        .filter(|(key, _)| key.starts_with('$'));
    let Some((key, operand)) = operator else {
        return Judgement::Holds(expected == actual);
    };
    let items = operand.as_array().map_or(&[][..], Vec::as_slice);
    let values = actual.as_array();
    let present = !actual.is_null();
    match key.as_str() {
        "$at_least" => Judgement::Holds(
            decimal(actual)
                .zip(decimal(operand))
                .is_some_and(|(value, bound)| value >= bound),
        ),
        "$contains" => Judgement::Holds(
            values.is_some_and(|values| items.iter().all(|item| values.contains(item))),
        ),
        "$excludes" => Judgement::Holds(
            values.is_some_and(|values| items.iter().all(|item| !values.contains(item))),
        ),
        "$present" => operand.as_bool().map_or_else(
            || Judgement::Unjudgeable("$present takes a boolean".to_owned()),
            |flag| Judgement::Holds(present == flag),
        ),
        "$capture" => Judgement::Holds(present),
        "$equals_file" => equals_file(operand, actual, repo),
        unknown => Judgement::UnknownMatcher(unknown.to_owned()),
    }
}

#[cfg(test)]
mod tests;

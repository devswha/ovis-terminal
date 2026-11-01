//! Recomputing a recorded assertion from its expected and actual values, using the
//! conformance runner's matcher operators.

use serde_json::Value;

fn decimal(value: &Value) -> Option<u128> {
    let text = value.as_str()?;
    (text == "0" || !text.starts_with('0')).then(|| text.parse().ok())?
}

/// Whether `actual` satisfies `expected` for the assertion named `check`, or `None` when
/// the record alone cannot tell (`$present`, `$capture` and `$equals_file` record no
/// comparable operand). `expect_failure/kind` compares only the failure kind.
pub(super) fn holds(check: &str, expected: &Value, actual: &Value) -> Option<bool> {
    if check == "expect_failure/kind" {
        return Some(expected.get("kind").is_some() && expected.get("kind") == actual.get("kind"));
    }
    let operator = expected
        .as_object()
        .filter(|object| object.len() == 1)
        .and_then(|object| object.iter().next())
        .filter(|(key, _)| key.starts_with('$'));
    let Some((key, operand)) = operator else {
        return Some(expected == actual);
    };
    let items = operand.as_array().map_or(&[][..], Vec::as_slice);
    let values = actual.as_array();
    match key.as_str() {
        "$at_least" => Some(
            decimal(actual)
                .zip(decimal(operand))
                .is_some_and(|(value, bound)| value >= bound),
        ),
        "$contains" => {
            Some(values.is_some_and(|values| items.iter().all(|item| values.contains(item))))
        }
        "$excludes" => {
            Some(values.is_some_and(|values| items.iter().all(|item| !values.contains(item))))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;

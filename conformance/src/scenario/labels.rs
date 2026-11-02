//! `$op:<label>` placeholders: one fresh `UUIDv7` per label per case run; `$value:<name>`
//! placeholders: the reply value an earlier `$capture` matcher recorded under that name.
//!
//! `$repeat:<count>:<text>` placeholders expand to `text` repeated `count` times, so a
//! scenario can send megabytes of input without storing them.

use std::collections::BTreeMap;

use serde_json::Value;

const PREFIX: &str = "$op:";
const VALUE: &str = "$value:";
const REPEAT: &str = "$repeat:";

/// Largest text one `$repeat:` placeholder expands to.
const REPEAT_MAX_BYTES: usize = 128 << 20;

/// `text` repeated `count` times for a `<count>:<text>` spec; none for a malformed or
/// oversized spec, which then stays as written.
fn repeated(spec: &str) -> Option<String> {
    let (count, text) = spec.split_once(':')?;
    let count: usize = count.parse().ok()?;
    let bytes = text.len().checked_mul(count)?;
    (!text.is_empty() && bytes <= REPEAT_MAX_BYTES).then(|| text.repeat(count))
}

/// Operation identities minted, and reply values captured, for one case.
#[derive(Debug, Default)]
pub(crate) struct Labels {
    ops: BTreeMap<String, String>,
    values: BTreeMap<String, Value>,
}

impl Labels {
    /// Records `value` for later `$value:<name>` strings.
    pub(crate) fn capture(&mut self, name: String, value: Value) {
        self.values.insert(name, value);
    }

    /// Copies `value`, replacing every `$op:<label>` string with that label's identity and
    /// every captured `$value:<name>` string with its value; an unknown name stays as is.
    pub(crate) fn resolve(&mut self, value: &Value) -> Value {
        match value {
            Value::String(text) => {
                if let Some(label) = text.strip_prefix(PREFIX) {
                    return Value::String(
                        self.ops
                            .entry(label.to_owned())
                            .or_insert_with(|| uuid::Uuid::now_v7().to_string())
                            .clone(),
                    );
                }
                if let Some(expanded) = text.strip_prefix(REPEAT).and_then(repeated) {
                    return Value::String(expanded);
                }
                text.strip_prefix(VALUE)
                    .and_then(|name| self.values.get(name))
                    .map_or_else(|| value.clone(), Value::clone)
            }
            Value::Array(items) => {
                Value::Array(items.iter().map(|item| self.resolve(item)).collect())
            }
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .map(|(key, item)| (key.clone(), self.resolve(item)))
                    .collect(),
            ),
            Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Labels;

    #[test]
    fn one_label_is_one_identity_and_distinct_labels_differ() {
        let mut labels = Labels::default();
        let first = labels.resolve(&json!({"op_id": "$op:a", "nested": ["$op:a", "$op:b", "x"]}));
        assert_eq!(first.pointer("/op_id"), first.pointer("/nested/0"));
        assert_ne!(first.pointer("/op_id"), first.pointer("/nested/1"));
        assert_eq!(first.pointer("/nested/2"), Some(&json!("x")));
        let again = labels.resolve(&json!("$op:a"));
        assert_eq!(first.pointer("/op_id"), Some(&again));
        let version = first
            .pointer("/op_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|text| uuid::Uuid::parse_str(text).ok())
            .map(|id| id.get_version_num());
        assert_eq!(version, Some(7));
    }

    #[test]
    fn captured_values_replace_their_placeholders_and_unknown_names_stay() {
        let mut labels = Labels::default();
        labels.capture("id".to_owned(), json!({"n": "1"}));
        let resolved = labels.resolve(&json!(["$value:id", "$value:other"]));
        assert_eq!(resolved, json!([{"n": "1"}, "$value:other"]));
    }

    #[test]
    fn repeat_placeholders_expand_and_malformed_ones_stay_as_written() {
        let mut labels = Labels::default();
        let resolved = labels.resolve(&json!({"text": "$repeat:3:ab", "one": "$repeat:1:q"}));
        assert_eq!(resolved, json!({"text": "ababab", "one": "q"}));
        let big = labels.resolve(&json!("$repeat:3145728:y"));
        assert_eq!(big.as_str().map(str::len), Some(3_145_728));
        for kept in [
            "$repeat:x:q",
            "$repeat:4:",
            "$repeat:4",
            "$repeat:999999999999:q",
        ] {
            assert_eq!(labels.resolve(&json!(kept)), json!(kept), "{kept}");
        }
    }
}

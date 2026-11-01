//! `$op:<label>` placeholders: one fresh `UUIDv7` per label per case run; `$value:<name>`
//! placeholders: the reply value an earlier `$capture` matcher recorded under that name.

use std::collections::BTreeMap;

use serde_json::Value;

const PREFIX: &str = "$op:";
const VALUE: &str = "$value:";

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
}

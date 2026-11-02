//! Symbolic identities bound at run time.
//!
//! `$op:<label>` becomes one fresh `UUIDv7` per label per case, `$value:<name>` the reply
//! value an earlier `$capture` recorded, and `$bind:<name>` a value the server profile
//! supplies (the fixture agent's process token, the pinned schema version). An unbound
//! `$value:` or `$bind:` is an error, never sent literally.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::error::ReplayError;

const OP: &str = "$op:";
const VALUE: &str = "$value:";
const BIND: &str = "$bind:";

/// The identities one case run has bound so far.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Bindings {
    ops: BTreeMap<String, String>,
    values: BTreeMap<String, Value>,
    server: BTreeMap<String, Value>,
}

impl Bindings {
    /// Bindings for one case on a server whose profile supplies `server`.
    #[must_use]
    pub const fn new(server: BTreeMap<String, Value>) -> Self {
        Self {
            ops: BTreeMap::new(),
            values: BTreeMap::new(),
            server,
        }
    }

    /// Records a captured reply value under `name`.
    pub fn capture(&mut self, name: &str, value: Value) {
        self.values.insert(name.to_owned(), value);
    }

    /// `value` with every symbol replaced.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayError::Unbound`] for a `$value:` or `$bind:` with no value.
    pub fn resolve(&mut self, value: &Value) -> Result<Value, ReplayError> {
        match value {
            Value::String(text) => self.symbol(text),
            Value::Array(items) => items
                .iter()
                .map(|item| self.resolve(item))
                .collect::<Result<_, _>>()
                .map(Value::Array),
            Value::Object(object) => object
                .iter()
                .map(|(key, item)| Ok((key.clone(), self.resolve(item)?)))
                .collect::<Result<_, ReplayError>>()
                .map(Value::Object),
            Value::Null | Value::Bool(_) | Value::Number(_) => Ok(value.clone()),
        }
    }

    fn symbol(&mut self, text: &str) -> Result<Value, ReplayError> {
        let unbound = || ReplayError::Unbound {
            symbol: text.to_owned(),
        };
        if let Some(label) = text.strip_prefix(OP) {
            let id = self
                .ops
                .entry(label.to_owned())
                .or_insert_with(|| uuid::Uuid::now_v7().to_string());
            return Ok(Value::String(id.clone()));
        }
        if let Some(name) = text.strip_prefix(VALUE) {
            return self.values.get(name).cloned().ok_or_else(unbound);
        }
        if let Some(name) = text.strip_prefix(BIND) {
            return self.server.get(name).cloned().ok_or_else(unbound);
        }
        Ok(Value::String(text.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use proptest::prelude::*;
    use serde_json::json;

    use super::Bindings;

    #[test]
    fn labels_values_and_server_bindings_resolve_and_unbound_symbols_fail() {
        let mut bindings =
            Bindings::new(BTreeMap::from([("token".to_owned(), json!("process-1"))]));
        bindings.capture("pane", json!("pane:7"));
        let resolved = bindings
            .resolve(&json!({"a": "$op:x", "b": ["$op:x", "$op:y"], "p": "$value:pane", "t": "$bind:token", "n": 3}))
            .expect("every symbol is bound");
        assert_eq!(resolved.pointer("/a"), resolved.pointer("/b/0"));
        assert_ne!(resolved.pointer("/a"), resolved.pointer("/b/1"));
        assert_eq!(
            (
                resolved.pointer("/p"),
                resolved.pointer("/t"),
                resolved.pointer("/n")
            ),
            (
                Some(&json!("pane:7")),
                Some(&json!("process-1")),
                Some(&json!(3))
            )
        );
        for unbound in ["$value:missing", "$bind:missing"] {
            let error = bindings.resolve(&json!([unbound])).expect_err("unbound");
            assert_eq!(error.kind(), "symbol_unbound");
        }
    }

    proptest! {
        #[test]
        fn plain_strings_pass_through_unchanged(text in "[a-z0-9:._ -]{0,24}") {
            let mut bindings = Bindings::default();
            let value = json!(text);
            let resolved = bindings.resolve(&value).ok();
            prop_assert_eq!(resolved, Some(value));
        }
    }
}

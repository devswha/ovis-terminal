//! Translation of herdr-era calls Ovis makes today into explicit-ID published methods.
//!
//! The map only renames: a known legacy field moves into the published request under
//! its new name, and every other field is carried to the top level unchanged so the
//! runtime's closed schema refuses it at its own path. No target is ever filled in;
//! a split whose pane is missing or misspelled is refused, never aimed at the focus.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::error::ReplayError;

/// Marker of `legacy.json`.
pub(crate) const FORMAT: &str = "ovis-legacy-map/1";

/// `legacy.json`: legacy method to its published translation.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LegacyMap {
    /// Always `ovis-legacy-map/1`.
    pub format: String,
    /// Translations by legacy method.
    pub methods: BTreeMap<String, Translation>,
}

/// How one legacy method becomes a published one.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Translation {
    /// Published method.
    pub method: String,
    /// Params member the renamed fields go into.
    pub into: String,
    /// Constant members of that object, such as the edit `op`.
    pub constant: Map<String, Value>,
    /// Legacy field to its published name and value renames.
    pub fields: BTreeMap<String, Field>,
}

/// One renamed field.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Field {
    /// Published field name.
    pub to: String,
    /// Legacy value to published value; values not listed are carried unchanged.
    #[serde(default)]
    pub values: BTreeMap<String, String>,
}

impl LegacyMap {
    /// The published `(method, params)` of legacy `method` with `params`; the operation
    /// identity is the step's own `$op:` label.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayError::UnknownLegacy`] for an unmapped method and
    /// [`ReplayError::LegacyParams`] for params that are not an object.
    pub fn translate(
        &self,
        step: &str,
        (method, params): (&str, &Value),
    ) -> Result<(String, Value), ReplayError> {
        let translation = self
            .methods
            .get(method)
            .ok_or_else(|| ReplayError::UnknownLegacy {
                method: method.to_owned(),
            })?;
        let legacy = params
            .as_object()
            .ok_or_else(|| ReplayError::LegacyParams {
                step: step.to_owned(),
            })?;
        let mut inner = translation.constant.clone();
        let mut outer = Map::new();
        outer.insert("op_id".to_owned(), Value::String(format!("$op:{step}")));
        for (key, value) in legacy {
            match translation.fields.get(key) {
                Some(field) => {
                    let renamed = value
                        .as_str()
                        .and_then(|text| field.values.get(text))
                        .map_or_else(|| value.clone(), |text| Value::String(text.clone()));
                    inner.insert(field.to.clone(), renamed);
                }
                None => {
                    outer.insert(key.clone(), value.clone());
                }
            }
        }
        outer.insert(translation.into.clone(), Value::Object(inner));
        Ok((translation.method.clone(), Value::Object(outer)))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::LegacyMap;

    fn map() -> LegacyMap {
        serde_json::from_value(json!({"format": "ovis-legacy-map/1", "methods": {"pane.split": {
            "method": "layout.edit", "into": "edit", "constant": {"op": "split_pane"},
            "fields": {"pane_id": {"to": "pane_id"},
                       "direction": {"to": "axis", "values": {"right": "horizontal", "down": "vertical"}}}}}}))
        .expect("the map parses")
    }

    #[test]
    fn known_fields_are_renamed_and_unknown_fields_reach_the_server_unchanged() {
        let (method, params) = map()
            .translate(
                "typo",
                (
                    "pane.split",
                    &json!({"pane_id_typo": "other-pane", "direction": "right"}),
                ),
            )
            .expect("translates");
        assert_eq!(method, "layout.edit");
        assert_eq!(
            params,
            json!({"op_id": "$op:typo", "edit": {"op": "split_pane", "axis": "horizontal"}, "pane_id_typo": "other-pane"})
        );
        let (_, explicit) = map()
            .translate(
                "ok",
                (
                    "pane.split",
                    &json!({"pane_id": "pane:1", "direction": "sideways"}),
                ),
            )
            .expect("translates");
        assert_eq!(explicit.pointer("/edit/pane_id"), Some(&json!("pane:1")));
        assert_eq!(
            explicit.pointer("/edit/axis"),
            Some(&json!("sideways")),
            "an unknown value is not guessed"
        );
    }

    #[test]
    fn unmapped_methods_and_non_object_params_are_refused() {
        let unmapped = map().translate("s", ("pane.focus", &json!({})));
        assert_eq!(
            unmapped.map_err(|e| e.kind()).err(),
            Some("legacy_untranslated")
        );
        let scalar = map().translate("s", ("pane.split", &json!("w1:p1")));
        assert_eq!(
            scalar.map_err(|e| e.kind()).err(),
            Some("legacy_untranslated")
        );
    }
}

//! The runtime contract recorded in `conformance/ideal-map.json`, regenerated from the
//! authoritative OpenRPC document: every method verbatim (params, result, errors, effect,
//! targeting), every error kind, event and notification, the protocol extensions, and a
//! digest per component schema. Verification compares it in both directions.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::matrix_build::sha256_hex;
use super::report::Finding;

/// Protocol-wide extensions recorded verbatim.
const EXTENSIONS: &[&str] = &[
    "x-capabilities",
    "x-compatibility",
    "x-error-data",
    "x-limits",
    "x-stream",
];

/// The complete contract of one OpenRPC document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Contract {
    pub(super) schema: String,
    pub(super) sha256: String,
    pub(super) generation: String,
    pub(super) methods: BTreeMap<String, Value>,
    pub(super) error_kinds: BTreeMap<String, Value>,
    pub(super) events: BTreeMap<String, Value>,
    pub(super) notifications: BTreeMap<String, Value>,
    pub(super) extensions: BTreeMap<String, Value>,
    /// Component name to the SHA-256 of its JSON schema.
    pub(super) components: BTreeMap<String, String>,
}

/// `entry` of an array under `pointer`, keyed by its `key` field, without that field.
fn keyed(schema: &Value, (pointer, key): (&str, &str)) -> BTreeMap<String, Value> {
    let entries = schema
        .pointer(pointer)
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    entries
        .iter()
        .filter_map(|entry| {
            let name = entry.get(key)?.as_str()?.to_owned();
            let mut rest: Map<String, Value> = entry.as_object()?.clone();
            rest.remove(key);
            Some((name, Value::Object(rest)))
        })
        .collect()
}

/// The contract of `schema_text`, recorded under the repository path `schema_path`.
pub(super) fn contract(
    schema_path: &str,
    schema_text: &str,
) -> Result<Contract, serde_json::Error> {
    let schema: Value = serde_json::from_str(schema_text)?;
    let components = schema
        .pointer("/components/schemas")
        .and_then(Value::as_object)
        .map(|schemas| {
            schemas
                .iter()
                .map(|(name, body)| (name.clone(), sha256_hex(body.to_string().as_bytes())))
                .collect()
        })
        .unwrap_or_default();
    let extensions = EXTENSIONS
        .iter()
        .filter_map(|name| Some(((*name).to_owned(), schema.get(*name)?.clone())))
        .collect();
    Ok(Contract {
        schema: schema_path.to_owned(),
        sha256: sha256_hex(schema_text.as_bytes()),
        generation: schema
            .pointer("/info/x-generation")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        methods: keyed(&schema, ("/methods", "name")),
        error_kinds: keyed(&schema, ("/x-error-kinds", "kind")),
        events: keyed(&schema, ("/x-events", "name")),
        notifications: keyed(&schema, ("/x-notifications", "name")),
        extensions,
        components,
    })
}

fn compare<T: PartialEq>(
    section: &str,
    (recorded, current): (&BTreeMap<String, T>, &BTreeMap<String, T>),
) -> Vec<Finding> {
    let mut found = Vec::new();
    for (name, value) in current {
        match recorded.get(name) {
            None => found.push(Finding::new(
                "contract-missing",
                format!("{section}/{name}"),
                "the schema has it and the recorded contract does not",
            )),
            Some(kept) if kept != value => found.push(Finding::new(
                "contract-drift",
                format!("{section}/{name}"),
                "the recorded entry differs from the schema",
            )),
            Some(_) => {}
        }
    }
    for name in recorded.keys().filter(|name| !current.contains_key(*name)) {
        found.push(Finding::new(
            "contract-removed",
            format!("{section}/{name}"),
            "the schema no longer contains it",
        ));
    }
    found
}

/// Every difference between the recorded contract and the one `current` regenerates:
/// an omission, a removal and a changed entry each fail.
pub(super) fn diff(recorded: &Contract, current: &Contract) -> Vec<Finding> {
    let mut found = Vec::new();
    found.extend(compare("methods", (&recorded.methods, &current.methods)));
    found.extend(compare(
        "error_kinds",
        (&recorded.error_kinds, &current.error_kinds),
    ));
    found.extend(compare("events", (&recorded.events, &current.events)));
    found.extend(compare(
        "notifications",
        (&recorded.notifications, &current.notifications),
    ));
    found.extend(compare(
        "extensions",
        (&recorded.extensions, &current.extensions),
    ));
    found.extend(compare(
        "components",
        (&recorded.components, &current.components),
    ));
    let header = (&recorded.schema, &recorded.sha256, &recorded.generation);
    if header != (&current.schema, &current.sha256, &current.generation) {
        found.push(Finding::new(
            "contract-stale",
            recorded.schema.clone(),
            format!(
                "recorded sha256 {} generation {}, the schema is {} generation {}",
                recorded.sha256, recorded.generation, current.sha256, current.generation
            ),
        ));
    }
    found
}

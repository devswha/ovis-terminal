//! Every name a guide row or example uses, checked against the published schema, the
//! schema the packed Bun client ships, the methods the binary serves and the fixtures.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::guide::{Guide, Row};

#[derive(Debug, Default)]
pub(super) struct Contract {
    pub(super) methods: BTreeSet<String>,
    pub(super) events: BTreeSet<String>,
    pub(super) notifications: BTreeSet<String>,
    pub(super) components: BTreeSet<String>,
    /// `capabilities.<key>` and `limits.<key>`.
    pub(super) capabilities: BTreeSet<String>,
    /// Methods `hello` lists on the assembled binary.
    pub(super) served: BTreeSet<String>,
    /// Legacy herdr methods the fixture legacy map translates.
    pub(super) legacy: BTreeSet<String>,
    /// `fixture:<case>` and `scenario:<family/name>` references that exist.
    pub(super) fixtures: BTreeSet<String>,
}

fn names(list: Option<&Value>, key: &str) -> BTreeSet<String> {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get(key).and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn keys(object: Option<&Value>) -> impl Iterator<Item = String> {
    object
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|map| map.keys().cloned())
}

impl Contract {
    pub(super) fn of_schema(schema: &Value) -> Self {
        let components = schema.pointer("/components/schemas");
        let properties = |name: &str| {
            components
                .and_then(|all| all.get(name))
                .and_then(|component| component.get("properties"))
        };
        let capabilities = keys(properties("Capabilities"))
            .map(|key| format!("capabilities.{key}"))
            .chain(keys(properties("Limits")).map(|key| format!("limits.{key}")))
            .collect();
        Self {
            methods: names(schema.get("methods"), "name"),
            events: names(schema.get("x-events"), "name"),
            notifications: names(schema.get("x-notifications"), "name"),
            components: keys(components).collect(),
            capabilities,
            ..Self::default()
        }
    }

    /// Methods and notifications the examples send or await that the schema lacks.
    fn example_names<'g>(&self, guide: &'g Guide) -> Vec<&'g str> {
        let mut unknown: BTreeSet<&str> = BTreeSet::new();
        for (_, case) in &guide.examples {
            for step in case
                .get("steps")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let (method, known) = step.get("call").map_or_else(
                    || {
                        step.get("legacy").map_or_else(
                            || (step.pointer("/event/method"), &self.notifications),
                            |legacy| (legacy.get("method"), &self.legacy),
                        )
                    },
                    |call| (call.get("method"), &self.methods),
                );
                let method = method.and_then(Value::as_str).unwrap_or_default();
                if !known.contains(method) {
                    unknown.insert(method);
                }
            }
        }
        unknown.into_iter().collect()
    }

    /// The report over `guide`; [`consistent`] is the report of a guide that names only
    /// what exists, lists served methods as served and unserved ones as gaps.
    pub(super) fn report(&self, guide: &Guide) -> Value {
        let served: BTreeSet<String> = self.methods.intersection(&self.served).cloned().collect();
        let unserved: BTreeSet<String> = self.methods.difference(&self.served).cloned().collect();
        let events: BTreeSet<String> = self.events.union(&self.notifications).cloned().collect();
        json!({
            "methods_not_served": unknown(guide, |row| &row.methods, &served),
            "unserved_now_served": unknown(guide, |row| &row.unserved, &unserved),
            "unknown_events": unknown(guide, |row| &row.events, &events),
            "unknown_capabilities": unknown(guide, |row| &row.capabilities, &self.capabilities),
            "unknown_identity": unknown(guide, |row| &row.identity, &self.components),
            "unknown_fixtures": unknown(guide, |row| &row.fixtures, &self.fixtures),
            "example_unknown_methods": self.example_names(guide),
        })
    }
}

/// Names one list of the rows uses that `known` lacks, sorted and deduplicated.
fn unknown<'g>(
    guide: &'g Guide,
    listed: fn(&Row) -> &Vec<String>,
    known: &BTreeSet<String>,
) -> Vec<&'g str> {
    guide
        .rows
        .iter()
        .flat_map(|row| listed(row).iter().map(String::as_str))
        .filter(|name| !known.contains(*name))
        .collect::<BTreeSet<&str>>()
        .into_iter()
        .collect()
}

pub(super) fn consistent() -> Value {
    json!({
        "methods_not_served": [],
        "unserved_now_served": [],
        "unknown_events": [],
        "unknown_capabilities": [],
        "unknown_identity": [],
        "unknown_fixtures": [],
        "example_unknown_methods": [],
    })
}

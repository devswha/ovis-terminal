//! `conformance/traceability.json` and `conformance/ideal-map.json`: their shapes and the
//! structural check that needs no plan (ledger coverage, predicates, schema contract).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ledgers::Ledgers;
use super::report::Finding;

pub(super) const TRACEABILITY_FORMAT: &str = "ovis-quality-traceability/1";
pub(super) const IDEAL_MAP_FORMAT: &str = "ovis-ideal-map/1";

/// The inputs a matrix was generated from, by digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Sources {
    pub(super) plan: String,
    pub(super) plan_sha256: String,
    pub(super) ideal_state_sha256: String,
    pub(super) gap_list_sha256: String,
    pub(super) command: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Owners {
    pub(super) todos: Vec<String>,
    pub(super) scenarios: Vec<String>,
}

/// Every IDEAL, GAP and EXCL id to its owning todos and their scenarios.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Traceability {
    pub(super) format: String,
    pub(super) source: Sources,
    pub(super) derivation: String,
    pub(super) ideals: BTreeMap<String, Owners>,
    pub(super) gaps: BTreeMap<String, Owners>,
    pub(super) exclusions: BTreeMap<String, Owners>,
}

/// The runtime contract surface, regenerated from the OpenRPC schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Contract {
    pub(super) schema: String,
    pub(super) sha256: String,
    pub(super) generation: String,
    pub(super) methods: Vec<String>,
    pub(super) error_kinds: Vec<String>,
    pub(super) events: Vec<String>,
    pub(super) notifications: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IdealEntry {
    pub(super) user: String,
    pub(super) predicate: String,
    pub(super) reason: String,
    pub(super) needs: Vec<String>,
    pub(super) gaps: Vec<String>,
    pub(super) todos: Vec<String>,
    pub(super) scenarios: Vec<String>,
    pub(super) platforms: Vec<String>,
    pub(super) windows: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GapEntry {
    pub(super) ideals: Vec<String>,
    pub(super) todos: Vec<String>,
    pub(super) scenarios: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExclusionEntry {
    pub(super) title: String,
    pub(super) todos: Vec<String>,
    pub(super) scenarios: Vec<String>,
}

/// One predicate per IDEAL row, every GAP and EXCL row, and the schema contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IdealMap {
    pub(super) format: String,
    pub(super) source: Sources,
    pub(super) contract: Contract,
    pub(super) ideals: BTreeMap<String, IdealEntry>,
    pub(super) gaps: BTreeMap<String, GapEntry>,
    pub(super) exclusions: BTreeMap<String, ExclusionEntry>,
}

fn names(schema: &Value, (pointer, field): (&str, &str)) -> Vec<String> {
    let mut found: Vec<String> = schema
        .pointer(pointer)
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter_map(|entry| entry.get(field).and_then(Value::as_str).map(str::to_owned))
        .collect();
    found.sort();
    found.dedup();
    found
}

/// The contract catalogs of `schema`, without path and digest.
pub(super) fn catalogs(schema: &Value) -> (String, [Vec<String>; 4]) {
    let generation = schema
        .pointer("/info/x-generation")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let lists = [
        names(schema, ("/methods", "name")),
        names(schema, ("/x-error-kinds", "kind")),
        names(schema, ("/x-events", "name")),
        names(schema, ("/x-notifications", "name")),
    ];
    (generation, lists)
}

fn coverage<T>(section: &BTreeMap<String, T>, (name, wanted): (&str, Vec<String>)) -> Vec<Finding> {
    let have: BTreeSet<&String> = section.keys().collect();
    let wanted_set: BTreeSet<&String> = wanted.iter().collect();
    let missing = wanted_set.difference(&have).map(|id| {
        Finding::new(
            "matrix-coverage",
            format!("{name}/{id}"),
            "ledger row is missing",
        )
    });
    let unknown = have.difference(&wanted_set).map(|id| {
        Finding::new(
            "matrix-coverage",
            format!("{name}/{id}"),
            "not a ledger row",
        )
    });
    missing.chain(unknown).collect()
}

/// Structural findings: complete ledger coverage, verbatim predicates, and a contract that
/// the current schema still contains (additions are compatible, removals are not).
pub(super) fn verify(
    maps: (&Traceability, &IdealMap),
    ledgers: &Ledgers,
    schema: &Value,
) -> Vec<Finding> {
    let (trace, map) = maps;
    let mut found = Vec::new();
    if trace.format != TRACEABILITY_FORMAT || map.format != IDEAL_MAP_FORMAT {
        found.push(Finding::new(
            "matrix-coverage",
            "format",
            "unexpected matrix format",
        ));
    }
    found.extend(coverage(
        &trace.ideals,
        ("traceability/ideals", ledgers.ideal_ids()),
    ));
    found.extend(coverage(
        &trace.gaps,
        ("traceability/gaps", ledgers.gap_ids()),
    ));
    found.extend(coverage(
        &trace.exclusions,
        ("traceability/exclusions", ledgers.exclusion_ids()),
    ));
    found.extend(coverage(
        &map.ideals,
        ("ideal-map/ideals", ledgers.ideal_ids()),
    ));
    found.extend(coverage(&map.gaps, ("ideal-map/gaps", ledgers.gap_ids())));
    found.extend(coverage(
        &map.exclusions,
        ("ideal-map/exclusions", ledgers.exclusion_ids()),
    ));
    for ideal in &ledgers.ideals {
        if map
            .ideals
            .get(&ideal.id)
            .is_some_and(|entry| entry.predicate != ideal.statement)
        {
            found.push(Finding::new(
                "predicate-drift",
                ideal.id.clone(),
                "predicate differs from the ledger statement",
            ));
        }
    }
    let (generation, current) = catalogs(schema);
    if generation != map.contract.generation {
        found.push(Finding::new(
            "contract-removed",
            "generation",
            format!("schema generation {generation}"),
        ));
    }
    let recorded = [
        &map.contract.methods,
        &map.contract.error_kinds,
        &map.contract.events,
        &map.contract.notifications,
    ];
    for (listed, now) in recorded.into_iter().zip(&current) {
        for name in listed.iter().filter(|name| !now.contains(name)) {
            found.push(Finding::new(
                "contract-removed",
                name.clone(),
                "the schema no longer contains it",
            ));
        }
    }
    found
}

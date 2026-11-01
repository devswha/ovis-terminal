//! `conformance/traceability.json` and `conformance/ideal-map.json`: their shapes and the
//! structural check that needs no plan (ledger coverage, predicates, schema contract).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::contract::{Contract, contract, diff};
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

/// Every row whose owning todos or scenarios are empty: an unowned row is uncovered.
fn ownership(trace: &Traceability, map: &IdealMap) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut check = |section: &str, id: &String, (todos, scenarios): (&[String], &[String])| {
        if todos.is_empty() || scenarios.is_empty() {
            found.push(Finding::new(
                "matrix-unowned",
                format!("{section}/{id}"),
                "no owning todo or no scenario",
            ));
        }
    };
    for (id, owned) in &trace.ideals {
        check("traceability/ideals", id, (&owned.todos, &owned.scenarios));
    }
    for (id, owned) in &trace.gaps {
        check("traceability/gaps", id, (&owned.todos, &owned.scenarios));
    }
    for (id, owned) in &trace.exclusions {
        check(
            "traceability/exclusions",
            id,
            (&owned.todos, &owned.scenarios),
        );
    }
    for (id, entry) in &map.ideals {
        check("ideal-map/ideals", id, (&entry.todos, &entry.scenarios));
    }
    for (id, entry) in &map.gaps {
        check("ideal-map/gaps", id, (&entry.todos, &entry.scenarios));
    }
    for (id, entry) in &map.exclusions {
        check("ideal-map/exclusions", id, (&entry.todos, &entry.scenarios));
    }
    found
}

/// Structural findings: complete ledger coverage, an owner and a scenario for every row,
/// verbatim predicates, and a recorded contract equal to the one the schema regenerates.
pub(super) fn verify(
    maps: (&Traceability, &IdealMap),
    ledgers: &Ledgers,
    schema_text: &str,
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
    found.extend(ownership(trace, map));
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
    match contract(&map.contract.schema, schema_text) {
        Ok(current) => found.extend(diff(&map.contract, &current)),
        Err(error) => found.push(Finding::new(
            "contract-stale",
            map.contract.schema.clone(),
            error.to_string(),
        )),
    }
    found
}

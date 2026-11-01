//! Generating both matrices from the plan, the ledgers and the OpenRPC schema.

use std::collections::BTreeMap;

use serde_json::Value;
use sha2::{Digest as _, Sha256};

use super::ledgers::Ledgers;
use super::matrix::{
    Contract, ExclusionEntry, GapEntry, IDEAL_MAP_FORMAT, IdealEntry, IdealMap, Owners, Sources,
    TRACEABILITY_FORMAT, Traceability, catalogs,
};
use super::plan::{Plan, Todo};

const DERIVATION: &str = "todos: every plan todo whose Delivers names the IDEAL, whose Closes names the GAP, or whose block text names the EXCL; scenarios: the --scenario ids those todos name, in todo order (all/all excluded)";
const PLATFORMS: &[&str] = &["linux", "macos"];
const WINDOWS: &str = "contract_smoke";

/// Lowercase hex SHA-256 of `bytes`.
pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        text.extend(
            [byte >> 4, byte & 0x0f]
                .map(|nibble| char::from(HEX.get(usize::from(nibble)).copied().unwrap_or(b'0'))),
        );
    }
    text
}

/// Everything one generation reads.
#[derive(Debug)]
pub(super) struct Inputs<'input> {
    pub(super) plan: &'input Plan,
    pub(super) ledgers: &'input Ledgers,
    pub(super) schema_path: &'input str,
    pub(super) schema_text: &'input str,
    pub(super) sources: Sources,
}

fn owners(plan: &Plan, owns: impl Fn(&Todo) -> bool) -> Owners {
    let mut owners = Owners::default();
    for todo in plan.todos.iter().filter(|todo| owns(todo)) {
        owners.todos.push(todo.number.to_string());
        for scenario in &todo.scenarios {
            if !owners.scenarios.contains(scenario) {
                owners.scenarios.push(scenario.clone());
            }
        }
    }
    owners
}

fn contract(inputs: &Inputs<'_>) -> Result<Contract, serde_json::Error> {
    let schema: Value = serde_json::from_str(inputs.schema_text)?;
    let (generation, [methods, error_kinds, events, notifications]) = catalogs(&schema);
    Ok(Contract {
        schema: inputs.schema_path.to_owned(),
        sha256: sha256_hex(inputs.schema_text.as_bytes()),
        generation,
        methods,
        error_kinds,
        events,
        notifications,
    })
}

fn traceability(inputs: &Inputs<'_>) -> Traceability {
    let (plan, ledgers) = (inputs.plan, inputs.ledgers);
    let section =
        |ids: Vec<String>, field: fn(&Todo) -> &Vec<String>| -> BTreeMap<String, Owners> {
            ids.into_iter()
                .map(|id| {
                    let owned = owners(plan, |todo| field(todo).contains(&id));
                    (id, owned)
                })
                .collect()
        };
    Traceability {
        format: TRACEABILITY_FORMAT.to_owned(),
        source: inputs.sources.clone(),
        derivation: DERIVATION.to_owned(),
        ideals: section(ledgers.ideal_ids(), |todo| &todo.delivers),
        gaps: section(ledgers.gap_ids(), |todo| &todo.closes),
        exclusions: section(ledgers.exclusion_ids(), |todo| &todo.excludes),
    }
}

/// Both matrices; fails only when the schema text is not JSON.
pub(super) fn build(inputs: &Inputs<'_>) -> Result<(Traceability, IdealMap), serde_json::Error> {
    let trace = traceability(inputs);
    let ledgers = inputs.ledgers;
    let mut ideals = BTreeMap::new();
    for ideal in &ledgers.ideals {
        let owned = trace.ideals.get(&ideal.id).cloned().unwrap_or_default();
        let gaps = ledgers
            .gaps
            .iter()
            .filter(|gap| gap.ideals.contains(&ideal.id))
            .map(|gap| gap.id.clone())
            .collect();
        let entry = IdealEntry {
            user: ideal.user.clone(),
            predicate: ideal.statement.clone(),
            reason: ideal.reason.clone(),
            needs: ideal.needs.clone(),
            gaps,
            todos: owned.todos,
            scenarios: owned.scenarios,
            platforms: PLATFORMS
                .iter()
                .map(|platform| (*platform).to_owned())
                .collect(),
            windows: WINDOWS.to_owned(),
        };
        ideals.insert(ideal.id.clone(), entry);
    }
    let mut gaps = BTreeMap::new();
    for gap in &ledgers.gaps {
        let owned = trace.gaps.get(&gap.id).cloned().unwrap_or_default();
        let entry = GapEntry {
            ideals: gap.ideals.clone(),
            todos: owned.todos,
            scenarios: owned.scenarios,
        };
        gaps.insert(gap.id.clone(), entry);
    }
    let mut exclusions = BTreeMap::new();
    for exclusion in &ledgers.exclusions {
        let owned = trace
            .exclusions
            .get(&exclusion.id)
            .cloned()
            .unwrap_or_default();
        let entry = ExclusionEntry {
            title: exclusion.title.clone(),
            todos: owned.todos,
            scenarios: owned.scenarios,
        };
        exclusions.insert(exclusion.id.clone(), entry);
    }
    let map = IdealMap {
        format: IDEAL_MAP_FORMAT.to_owned(),
        source: inputs.sources.clone(),
        contract: contract(inputs)?,
        ideals,
        gaps,
        exclusions,
    };
    Ok((trace, map))
}

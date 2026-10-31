//! `docs/design/traceability.json` maps every ledger id to its owning todos and scenarios;
//! these tests tie it to the ledgers and to the registered scenario files.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::ids::ScenarioId;
use crate::scenario::load::{Repo, registered};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Traceability {
    format: String,
    source: String,
    derivation: String,
    ideals: BTreeMap<String, Owners>,
    gaps: BTreeMap<String, Owners>,
    exclusions: BTreeMap<String, Owners>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Owners {
    todos: Vec<String>,
    scenarios: Vec<ScenarioId>,
}

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn load() -> Result<Traceability, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(
        Repo::of_build()
            .root()
            .join("docs/design/traceability.json"),
    )?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Ids of the ledger's table rows, each starting `| <prefix>-NNN`.
fn ledger_ids(
    document: &str,
    prefix: &str,
) -> Result<BTreeSet<String>, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(Repo::of_build().root().join(document))?;
    let marker = format!("| {prefix}-");
    Ok(text
        .lines()
        .filter_map(|line| line.strip_prefix(&marker))
        .filter_map(|rest| rest.get(..3))
        .filter(|digits| digits.bytes().all(|byte| byte.is_ascii_digit()))
        .map(|digits| format!("{prefix}-{digits}"))
        .collect())
}

fn expected(prefix: &str, count: u16) -> BTreeSet<String> {
    (1..=count).map(|n| format!("{prefix}-{n:03}")).collect()
}

#[test]
fn every_ledger_id_is_mapped_exactly_once_to_known_todos() -> TestResult {
    let map = load()?;
    assert_eq!(map.format, "ovis-traceability/1");
    assert!(
        Repo::of_build().root().join(&map.source).is_file(),
        "{}",
        map.source
    );
    assert_ne!(map.derivation, "");
    let sections = [
        (&map.ideals, "IDEAL", 117, "docs/design/ideal-state.md"),
        (&map.gaps, "GAP", 150, "docs/design/gap-list.md"),
        (&map.exclusions, "EXCL", 27, "docs/design/ideal-state.md"),
    ];
    for (section, prefix, count, ledger) in sections {
        let keys: BTreeSet<String> = section.keys().cloned().collect();
        assert_eq!(keys, expected(prefix, count), "{prefix} ids");
        assert_eq!(
            ledger_ids(ledger, prefix)?,
            keys,
            "{prefix} ids differ from {ledger}"
        );
        for (id, owners) in section {
            assert!(!owners.todos.is_empty(), "{id} has no owning todo");
            assert!(!owners.scenarios.is_empty(), "{id} has no scenario");
            for todo in &owners.todos {
                let number: u16 = todo.parse()?;
                assert!(
                    (1..=70).contains(&number) && todo == &number.to_string(),
                    "{id}: todo {todo}"
                );
            }
            let unique: BTreeSet<&ScenarioId> = owners.scenarios.iter().collect();
            assert_eq!(
                unique.len(),
                owners.scenarios.len(),
                "{id} repeats a scenario"
            );
        }
    }
    Ok(())
}

#[test]
fn every_registered_scenario_is_listed_under_each_id_it_declares() -> TestResult {
    let map = load()?;
    let files = registered(&Repo::of_build())?;
    assert!(!files.is_empty());
    for file in files {
        let declared = file
            .delivers
            .iter()
            .map(|id| (&map.ideals, id))
            .chain(file.closes.iter().map(|id| (&map.gaps, id)))
            .chain(file.excludes.iter().map(|id| (&map.exclusions, id)));
        for (section, id) in declared {
            let owners = section
                .get(id)
                .ok_or_else(|| format!("{}: unknown id {id}", file.scenario))?;
            assert!(
                owners.scenarios.contains(&file.scenario),
                "{id} does not list {}",
                file.scenario
            );
            assert!(
                owners.todos.contains(&file.todo.to_string()),
                "{id} does not list todo {}",
                file.todo
            );
        }
    }
    Ok(())
}

//! The reverse direction: a scenario the ledger lists as an owner of a row still
//! declares that row. Rows recorded in `docs/design/traceability-unclaimed.json` are
//! known exceptions, each naming the scenario's own todo; any other unclaimed row fails,
//! and so does an exception that no longer applies, so the list only shrinks.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use super::{Owners, TestResult, Traceability, load};
use crate::scenario::load::Repo;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Exceptions {
    format: String,
    derivation: String,
    entries: Vec<Exception>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Exception {
    id: String,
    scenario: String,
    owner_todo: String,
    reason: String,
}

/// One ledger row and one scenario it lists.
type Row = (String, String);

fn sections(map: &Traceability) -> [(&BTreeMap<String, Owners>, &'static str); 3] {
    [
        (&map.ideals, "delivers"),
        (&map.gaps, "closes"),
        (&map.exclusions, "excludes"),
    ]
}

/// Every row whose listed scenario file exists below `scenarios` but does not declare
/// the row's id in the field of its section. A scenario not written yet is skipped.
fn unclaimed(map: &Traceability, scenarios: &Path) -> Result<BTreeSet<Row>, Box<dyn Error>> {
    let mut found = BTreeSet::new();
    for (section, field) in sections(map) {
        for (id, owners) in section {
            for scenario in &owners.scenarios {
                let path = scenarios.join(scenario.relative_path());
                if !path.is_file() {
                    continue;
                }
                let value: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
                let claims = value
                    .get(field)
                    .and_then(Value::as_array)
                    .is_some_and(|ids| ids.iter().any(|claimed| claimed.as_str() == Some(id)));
                if !claims {
                    found.insert((id.clone(), scenario.to_string()));
                }
            }
        }
    }
    Ok(found)
}

fn todo_of(scenarios: &Path, scenario: &str) -> Result<String, Box<dyn Error>> {
    let value: Value =
        serde_json::from_slice(&std::fs::read(scenarios.join(format!("{scenario}.json")))?)?;
    Ok(match value.get("todo") {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    })
}

#[test]
fn every_listed_owner_claims_its_row_except_the_recorded_ones() -> TestResult {
    let repo = Repo::of_build();
    let path = repo.root().join("docs/design/traceability-unclaimed.json");
    let exceptions: Exceptions = serde_json::from_slice(&std::fs::read(path)?)?;
    assert_eq!(exceptions.format, "ovis-traceability-unclaimed/1");
    assert_ne!(exceptions.derivation, "");
    let mut recorded = BTreeSet::new();
    for entry in &exceptions.entries {
        let owner = todo_of(repo.scenarios_dir(), &entry.scenario)?;
        assert_eq!(entry.owner_todo, owner, "{} {}", entry.id, entry.scenario);
        assert!(entry.reason.contains(&entry.id), "{}: reason", entry.id);
        recorded.insert((entry.id.clone(), entry.scenario.clone()));
    }
    let found = unclaimed(&load()?, repo.scenarios_dir())?;
    let new: Vec<&Row> = found.difference(&recorded).collect();
    assert_eq!(
        new,
        Vec::<&Row>::new(),
        "listed owners that do not claim their row"
    );
    let resolved: Vec<&Row> = recorded.difference(&found).collect();
    assert_eq!(
        resolved,
        Vec::<&Row>::new(),
        "exceptions that no longer apply"
    );
    Ok(())
}

#[test]
fn a_listed_owner_that_dropped_the_row_is_flagged() -> TestResult {
    let dir = std::env::temp_dir().join(format!("ovc-unclaimed-{}", std::process::id()));
    let family = dir.join("zz");
    std::fs::create_dir_all(&family)?;
    std::fs::write(
        family.join("keeps.json"),
        r#"{"todo": 7, "closes": ["GAP-050"]}"#,
    )?;
    std::fs::write(
        family.join("dropped.json"),
        r#"{"todo": 7, "closes": ["GAP-049"]}"#,
    )?;
    let owners = serde_json::json!({
        "todos": ["7"], "scenarios": ["zz/keeps", "zz/dropped", "zz/unwritten"]
    });
    let map: Traceability = serde_json::from_value(serde_json::json!({
        "format": "ovis-traceability/1", "source": "s", "derivation": "d",
        "ideals": {}, "gaps": {"GAP-050": owners}, "exclusions": {}
    }))?;
    let found = unclaimed(&map, &dir)?;
    std::fs::remove_dir_all(&dir)?;
    let expected: BTreeSet<Row> = [("GAP-050".to_owned(), "zz/dropped".to_owned())].into();
    assert_eq!(found, expected);
    Ok(())
}

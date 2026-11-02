//! `predicates.json`: one entry per IDEAL predicate and per exclusion, bound to the
//! ledger by digest and to the lane scenarios and replay cases that prove it.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::case::{Case, CaseId};
use crate::expect::Check;
use crate::hash;

/// Marker of `predicates.json`.
pub(crate) const FORMAT: &str = "ovis-predicates/1";

/// The scenario this fixture set registers as.
pub(crate) const OWN_SCENARIO: &str = "client/fixtures";

/// `predicates.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Predicates {
    /// Always `ovis-predicates/1`.
    pub format: String,
    /// One entry per IDEAL row of the ideal map.
    pub ideals: Vec<Predicate>,
    /// One entry per EXCL row of the ideal map.
    pub exclusions: Vec<Predicate>,
    /// Mapped scenarios whose files do not exist yet, with their owning todo.
    pub pending: BTreeMap<String, String>,
}

/// One ledger row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Predicate {
    /// `IDEAL-NNN` or `EXCL-NNN`.
    pub id: String,
    /// SHA-256 of the IDEAL predicate text, or of the exclusion title.
    pub sha256: String,
    /// Lane scenarios that prove it, exactly as the ideal map lists them.
    pub scenarios: Vec<String>,
    /// Replay cases of this set that prove it; required when `client/fixtures` is listed.
    pub replay: Vec<CaseId>,
}

/// What a scenario identity resolves to in the checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScenarioState {
    /// A registered, runner-executable scenario file.
    Executable,
    /// A declaration listed in `declared.json`, not yet executable.
    Declared,
    /// No file.
    Absent,
}

/// The parts of `conformance/ideal-map.json` the predicates bind to.
#[derive(Debug, Deserialize)]
pub(crate) struct IdealMap {
    pub(crate) ideals: BTreeMap<String, Row>,
    pub(crate) exclusions: BTreeMap<String, Row>,
}

/// One ideal-map row; `predicate` for IDEAL rows, `title` for EXCL rows.
#[derive(Debug, Deserialize)]
pub(crate) struct Row {
    #[serde(default)]
    pub(crate) predicate: Option<String>,
    #[serde(default)]
    pub(crate) title: Option<String>,
    pub(crate) scenarios: Vec<String>,
}

impl Row {
    pub(crate) fn digest(&self) -> String {
        let text = self
            .predicate
            .as_deref()
            .or(self.title.as_deref())
            .unwrap_or_default();
        hash::sha256(text.as_bytes())
    }
}

/// What the audit compares the predicates with besides the ideal map.
#[derive(Clone, Copy)]
pub(crate) struct Evidence<'a> {
    /// The fixture set's cases.
    pub(crate) cases: &'a [Case],
    /// How the checkout resolves a scenario identity.
    pub(crate) resolve: &'a dyn Fn(&str) -> ScenarioState,
}

/// The rows a case claims in one section.
fn claims<'c>(case: &'c Case, section: &str) -> &'c [String] {
    if section == "ideals" {
        &case.ideals
    } else {
        &case.excludes
    }
}

impl Predicates {
    /// Checks the predicates against the ideal map, the checkout's scenarios and the
    /// cases: same rows and digests, resolvable scenarios, and replay links both ways.
    pub(crate) fn audit(&self, map: &IdealMap, evidence: Evidence<'_>) -> Vec<Check> {
        let mut checks = Vec::new();
        for (section, rows, entries) in [
            ("ideals", &map.ideals, &self.ideals),
            ("exclusions", &map.exclusions, &self.exclusions),
        ] {
            let listed: Vec<&String> = entries.iter().map(|entry| &entry.id).collect();
            let expected: Vec<&String> = rows.keys().collect();
            checks.push(Check::equal(
                format!("predicates/{section}/ids"),
                json!(expected),
                json!(listed),
            ));
            for entry in entries {
                checks.extend(self.entry(entry, (rows.get(&entry.id), section, evidence)));
            }
        }
        checks
    }

    fn entry(
        &self,
        entry: &Predicate,
        (row, section, evidence): (Option<&Row>, &str, Evidence<'_>),
    ) -> Vec<Check> {
        let Evidence { cases, resolve } = evidence;
        let scope = format!("predicates/{}", entry.id);
        let mut checks = vec![
            Check::equal(
                format!("{scope}/sha256"),
                json!(row.map(Row::digest)),
                json!(entry.sha256),
            ),
            Check::equal(
                format!("{scope}/scenarios"),
                json!(row.map(|row| &row.scenarios)),
                json!(entry.scenarios),
            ),
        ];
        for scenario in &entry.scenarios {
            let pending = self.pending.contains_key(scenario);
            let resolved = match resolve(scenario) {
                ScenarioState::Executable | ScenarioState::Declared => !pending,
                ScenarioState::Absent => pending,
            };
            checks.push(Check::equal(
                format!("{scope}/scenario/{scenario}"),
                json!(true),
                json!(resolved),
            ));
        }
        let owned = entry
            .scenarios
            .iter()
            .any(|scenario| scenario == OWN_SCENARIO);
        checks.push(Check::equal(
            format!("{scope}/replayed"),
            json!(owned),
            json!(!entry.replay.is_empty()),
        ));
        let linked: BTreeSet<&CaseId> = entry.replay.iter().collect();
        let claimed: BTreeSet<&CaseId> = cases
            .iter()
            .filter(|case| claims(case, section).contains(&entry.id))
            .map(|case| &case.id)
            .collect();
        checks.push(Check::equal(
            format!("{scope}/replay"),
            json!(claimed),
            json!(linked),
        ));
        checks
    }

    /// The predicates the ideal map implies, keeping this file's replay links and pending
    /// owners for rows and scenarios that still exist.
    pub(crate) fn derived(&self, map: &IdealMap, absent: &dyn Fn(&str) -> bool) -> Self {
        let keep = |rows: &BTreeMap<String, Row>, old: &[Predicate]| -> Vec<Predicate> {
            rows.iter()
                .map(|(id, row)| Predicate {
                    id: id.clone(),
                    sha256: row.digest(),
                    scenarios: row.scenarios.clone(),
                    replay: old
                        .iter()
                        .find(|entry| &entry.id == id)
                        .map(|entry| entry.replay.clone())
                        .unwrap_or_default(),
                })
                .collect()
        };
        let ideals = keep(&map.ideals, &self.ideals);
        let exclusions = keep(&map.exclusions, &self.exclusions);
        let pending = ideals
            .iter()
            .chain(&exclusions)
            .flat_map(|entry| entry.scenarios.iter())
            .filter(|scenario| absent(scenario))
            .map(|scenario| {
                (
                    scenario.clone(),
                    self.pending.get(scenario).cloned().unwrap_or_default(),
                )
            })
            .collect();
        Self {
            format: FORMAT.to_owned(),
            ideals,
            exclusions,
            pending,
        }
    }
}

#[cfg(test)]
pub(crate) fn row(predicate: &str, scenarios: &[&str]) -> Row {
    Row {
        predicate: Some(predicate.to_owned()),
        title: None,
        scenarios: scenarios.iter().map(|s| (*s).to_owned()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{Evidence, IdealMap, Predicates, ScenarioState, row};

    #[test]
    fn digests_scenarios_pending_and_replay_links_each_fail_when_they_drift() {
        let map = IdealMap {
            ideals: BTreeMap::from([(
                "IDEAL-001".to_owned(),
                row("endpoints", &["a/b", "client/fixtures", "c/later"]),
            )]),
            exclusions: BTreeMap::new(),
        };
        let resolve = |s: &str| match s {
            "c/later" => ScenarioState::Absent,
            _ => ScenarioState::Executable,
        };
        let empty = Predicates {
            format: String::new(),
            ideals: Vec::new(),
            exclusions: Vec::new(),
            pending: BTreeMap::new(),
        };
        let mut good = empty.derived(&map, &|s| resolve(s) == ScenarioState::Absent);
        good.pending.insert("c/later".to_owned(), "65".to_owned());
        let case: crate::case::Case = serde_json::from_value(json!({"format": "ovis-replay/1", "id": "k",
            "server": "assembled", "kind": "happy", "ideals": ["IDEAL-001"], "gaps": [], "excludes": [],
            "adversarial": [], "steps": []})).expect("case");
        if let Some(entry) = good.ideals.first_mut() {
            entry.replay = vec![case.id.clone()];
        }
        let cases = [case];
        let evidence = Evidence {
            cases: &cases,
            resolve: &resolve,
        };
        assert!(good.audit(&map, evidence).iter().all(|c| c.passed));

        let mut drifted = good.clone();
        if let Some(entry) = drifted.ideals.first_mut() {
            entry.sha256 = "0".repeat(64);
            entry.replay.clear();
        }
        drifted.pending.clear();
        let failed: Vec<String> = drifted
            .audit(&map, evidence)
            .into_iter()
            .filter(|c| !c.passed)
            .map(|c| c.check)
            .collect();
        assert_eq!(
            failed,
            [
                "predicates/IDEAL-001/sha256",
                "predicates/IDEAL-001/scenario/c/later",
                "predicates/IDEAL-001/replayed",
                "predicates/IDEAL-001/replay",
            ]
        );
    }
}

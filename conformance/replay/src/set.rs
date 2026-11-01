//! A loaded fixture set and its audit.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::json;

use crate::case::{self, Adversarial, Case, CaseId};
use crate::error::ReplayError;
use crate::expect::Check;
use crate::hash;
use crate::legacy::{self, LegacyMap};
use crate::manifest::{self, Manifest};
use crate::predicates::{self, Evidence, IdealMap, Predicates, ScenarioState};
use crate::receipt::Receipt;

/// The ideal map the predicates bind to, relative to the repository.
pub(crate) const IDEAL_MAP: &str = "conformance/ideal-map.json";
/// Marker of `receipts.json`.
pub(crate) const RECEIPTS_FORMAT: &str = "ovis-receipts/1";
const CASES: &str = "cases/";

/// `receipts.json`: the normalized receipt every client must reproduce, per case.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipts {
    format: String,
    cases: BTreeMap<CaseId, Receipt>,
}

/// Every file of a fixture directory, parsed.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FixtureSet {
    /// The fixture directory.
    pub dir: PathBuf,
    /// `manifest.json`.
    pub manifest: Manifest,
    /// `legacy.json`.
    pub legacy: LegacyMap,
    /// `predicates.json`.
    pub predicates: Predicates,
    /// The expected receipt of each case.
    pub receipts: BTreeMap<CaseId, Receipt>,
    /// Every case the manifest lists, in manifest order.
    pub cases: Vec<Case>,
}

fn load<T: serde::de::DeserializeOwned>(
    dir: &Path,
    name: &str,
) -> Result<(PathBuf, T), ReplayError> {
    let path = dir.join(name);
    let value = hash::json(&path)?;
    Ok((path, value))
}

impl FixtureSet {
    /// Parses the fixture set in `dir`.
    ///
    /// # Errors
    ///
    /// Returns the first unreadable, malformed or wrongly versioned file.
    pub fn load(dir: &Path) -> Result<Self, ReplayError> {
        let (path, manifest): (_, Manifest) = load(dir, "manifest.json")?;
        hash::format(&path, (manifest::FORMAT, &manifest.format))?;
        let (path, legacy): (_, LegacyMap) = load(dir, "legacy.json")?;
        hash::format(&path, (legacy::FORMAT, &legacy.format))?;
        let (path, predicates): (_, Predicates) = load(dir, "predicates.json")?;
        hash::format(&path, (predicates::FORMAT, &predicates.format))?;
        let (path, receipts): (_, Receipts) = load(dir, "receipts.json")?;
        hash::format(&path, (RECEIPTS_FORMAT, &receipts.format))?;
        let mut cases = Vec::new();
        for pin in manifest
            .files
            .iter()
            .filter(|pin| pin.path.starts_with(CASES))
        {
            let (path, case): (_, Case) = load(dir, &pin.path)?;
            hash::format(&path, (case::FORMAT, &case.format))?;
            cases.push(case);
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            manifest,
            legacy,
            predicates,
            receipts: receipts.cases,
            cases,
        })
    }

    /// Every check of the set itself: pins, listed cases, adversarial coverage, receipts
    /// and the predicate ledger against the ideal map and the scenarios `resolve` sees.
    ///
    /// # Errors
    ///
    /// Returns the read failure of the fixture directory or the ideal map.
    pub fn audit(
        &self,
        repo: &Path,
        resolve: &dyn Fn(&str) -> ScenarioState,
    ) -> Result<Vec<Check>, ReplayError> {
        let mut checks = self.manifest.audit((repo, &self.dir));
        let generation = &self.manifest.generation;
        let canonical = generation
            .parse::<u64>()
            .is_ok_and(|n| &n.to_string() == generation);
        checks.push(Check::equal(
            "manifest/generation".to_owned(),
            json!(true),
            json!(canonical),
        ));
        checks.push(Check::equal(
            "cases/listed".to_owned(),
            json!(self.case_files()?),
            json!(self.listed()),
        ));
        let covered: BTreeSet<Adversarial> = self
            .cases
            .iter()
            .flat_map(|case| case.adversarial.iter().copied())
            .collect();
        checks.push(Check::equal(
            "adversarial/classes".to_owned(),
            json!(Adversarial::ALL),
            json!(covered),
        ));
        let ids: BTreeSet<&CaseId> = self.cases.iter().map(|case| &case.id).collect();
        let receipted: BTreeSet<&CaseId> = self.receipts.keys().collect();
        checks.push(Check::equal(
            "receipts/cases".to_owned(),
            json!(ids),
            json!(receipted),
        ));
        let map: IdealMap = hash::json(&repo.join(IDEAL_MAP))?;
        checks.extend(self.predicates.audit(
            &map,
            Evidence {
                cases: &self.cases,
                resolve,
            },
        ));
        Ok(checks)
    }

    fn listed(&self) -> Vec<String> {
        let mut listed: Vec<String> = self
            .manifest
            .files
            .iter()
            .filter(|pin| pin.path.starts_with(CASES))
            .map(|pin| pin.path.clone())
            .collect();
        listed.sort();
        listed
    }

    /// Every file below `cases/`, so an unlisted case cannot hide from the pins.
    fn case_files(&self) -> Result<Vec<String>, ReplayError> {
        let dir = self.dir.join(CASES);
        let io = |source| ReplayError::Io {
            operation: "list",
            path: dir.clone(),
            source,
        };
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&dir).map_err(io)? {
            let name = entry.map_err(io)?.file_name();
            files.push(format!("{CASES}{}", name.to_string_lossy()));
        }
        files.sort();
        Ok(files)
    }
}

//! `manifest.json`: the fixture set's version, pins and server profiles.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::case::Server;
use crate::error::ReplayError;
use crate::expect::Check;
use crate::hash;

/// Marker of `manifest.json`.
pub(crate) const FORMAT: &str = "ovis-consumer-fixtures/1";

/// The versioned head of a fixture set.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Manifest {
    /// Always `ovis-consumer-fixtures/1`.
    pub format: String,
    /// Protocol generation the fixtures speak, as a decimal string.
    pub generation: String,
    /// The published schema the fixtures were written against, relative to the repository.
    pub schema: Pin,
    /// Ledgers the predicates were derived from, relative to the repository.
    pub sources: Vec<Pin>,
    /// Every fixture file, relative to the fixture directory.
    pub files: Vec<Pin>,
    /// What each server supplies to `$bind:` symbols.
    pub servers: BTreeMap<Server, ServerProfile>,
}

/// A file and the SHA-256 it must have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Pin {
    /// Path, `/`-separated.
    pub path: String,
    /// Lowercase hex SHA-256 of its bytes.
    pub sha256: String,
}

/// Values a server supplies to `$bind:` symbols.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ServerProfile {
    /// Symbol name to value.
    pub bindings: BTreeMap<String, Value>,
}

impl Pin {
    /// A pin of the file at `base/path` as it is now.
    pub(crate) fn of(base: &Path, path: &str) -> Result<Self, ReplayError> {
        Ok(Self {
            path: path.to_owned(),
            sha256: hash::sha256(&hash::read(&base.join(path))?),
        })
    }

    /// Whether the file at `base/path` still has the pinned digest.
    pub(crate) fn audit(&self, base: &Path) -> Check {
        let actual = hash::read(&base.join(&self.path)).map_or_else(
            |error| Value::String(format!("unreadable: {}", error.kind())),
            |bytes| Value::String(hash::sha256(&bytes)),
        );
        Check::equal(
            format!("pin/{}", self.path),
            Value::String(self.sha256.clone()),
            actual,
        )
    }
}

impl Manifest {
    /// Every pin checked against the repository at `repo` and the fixtures in `dir`.
    #[must_use]
    pub fn audit(&self, (repo, dir): (&Path, &Path)) -> Vec<Check> {
        let mut checks = vec![self.schema.audit(repo)];
        checks.extend(self.sources.iter().map(|pin| pin.audit(repo)));
        checks.extend(self.files.iter().map(|pin| pin.audit(dir)));
        checks
    }

    /// The `$bind:` values of `server`, plus `schema_version` read from the pinned schema
    /// and the fixture `generation`.
    ///
    /// # Errors
    ///
    /// Returns the read or parse failure of the schema.
    pub fn bindings(
        &self,
        (server, repo): (Server, &Path),
    ) -> Result<BTreeMap<String, Value>, ReplayError> {
        let schema: Value = hash::json(&repo.join(&self.schema.path))?;
        let mut bindings = self
            .servers
            .get(&server)
            .map(|profile| profile.bindings.clone())
            .unwrap_or_default();
        if let Some(version) = schema.pointer("/info/version") {
            bindings.insert("schema_version".to_owned(), version.clone());
        }
        bindings.insert(
            "generation".to_owned(),
            Value::String(self.generation.clone()),
        );
        Ok(bindings)
    }
}

#[cfg(test)]
mod tests {
    use super::Pin;

    #[test]
    fn a_pin_holds_only_for_the_digest_of_the_bytes_on_disk()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!("ovis-t64-pin-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("f.json"), b"abc")?;
        let pinned = Pin::of(&dir, "f.json")?;
        assert_eq!(
            pinned.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(pinned.audit(&dir).passed);
        std::fs::write(dir.join("f.json"), b"abd")?;
        let drifted = pinned.audit(&dir);
        assert!(!drifted.passed, "{drifted:?}");
        assert_ne!(drifted.actual, drifted.expected);
        std::fs::remove_file(dir.join("f.json"))?;
        assert!(
            !pinned.audit(&dir).passed,
            "a missing file never matches its pin"
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}

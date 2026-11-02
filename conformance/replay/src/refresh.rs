//! Re-deriving `predicates.json` from the ideal map and re-pinning the manifest after an
//! intended change of the schema, the ledgers or a fixture file.

use std::path::Path;

use serde::Serialize;

use crate::error::ReplayError;
use crate::hash;
use crate::manifest::{Manifest, Pin};
use crate::predicates::{IdealMap, Predicates};
use crate::set::IDEAL_MAP;

fn write<T: Serialize>(path: &Path, value: &T) -> Result<(), ReplayError> {
    let mut text = serde_json::to_string_pretty(value).map_err(|source| ReplayError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    text.push('\n');
    std::fs::write(path, text).map_err(|source| ReplayError::Io {
        operation: "write",
        path: path.to_path_buf(),
        source,
    })
}

/// Rewrites `predicates.json` and `manifest.json` in `dir` against the repository at
/// `repo`; `absent` tells which scenario identities have no file yet.
///
/// # Errors
///
/// Returns the first unreadable, malformed or unwritable file.
pub fn refresh(
    repo: &Path,
    (dir, absent): (&Path, &dyn Fn(&str) -> bool),
) -> Result<Manifest, ReplayError> {
    let map: IdealMap = hash::json(&repo.join(IDEAL_MAP))?;
    let old: Predicates = hash::json(&dir.join("predicates.json"))?;
    write(&dir.join("predicates.json"), &old.derived(&map, absent))?;
    let mut manifest: Manifest = hash::json(&dir.join("manifest.json"))?;
    manifest.schema = Pin::of(repo, &manifest.schema.path)?;
    manifest.sources = manifest
        .sources
        .iter()
        .map(|pin| Pin::of(repo, &pin.path))
        .collect::<Result<_, _>>()?;
    manifest.files = manifest
        .files
        .iter()
        .map(|pin| Pin::of(dir, &pin.path))
        .collect::<Result<_, _>>()?;
    write(&dir.join("manifest.json"), &manifest)?;
    Ok(manifest)
}

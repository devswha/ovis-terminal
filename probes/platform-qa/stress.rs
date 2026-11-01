//! Stress lanes: `<lane>.json` plus `<lane>.sh` under `probes/platform-qa/stress/`.
//!
//! The dispatch workflow resolves one lane with `resolve.py`; this module is the same
//! rule set in Rust so a broken or half-added lane fails the test suite before CI.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::error::QaError;

/// Format marker of a lane manifest.
pub const LANE_FORMAT: &str = "ovis-stress-lane/1";

/// Setup a lane needs before its script runs.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Setup {
    /// The pinned MSVC toolset and linker.
    Msvc,
    /// Bun for the TypeScript client.
    Bun,
    /// The CI-owned Windows service account and binary location.
    WindowsServiceAccount,
}

/// One runner the lane's script runs on.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Variant {
    /// The runner image label.
    pub runs_on: String,
    /// Xcode version to pin, empty for none.
    pub xcode: String,
    /// Value of `STRESS_VARIANT` for the script, empty for none.
    pub variant: String,
}

/// One lane manifest.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StressLane {
    /// Always [`LANE_FORMAT`].
    pub format: String,
    /// Runners, one job each.
    pub variants: Vec<Variant>,
    /// Job timeout in minutes.
    pub timeout_minutes: u16,
    /// Setup steps the job runs first.
    pub setup: Vec<Setup>,
}

fn lane_name(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let grammar = stem
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric())
        && stem
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    grammar.then(|| stem.to_owned())
}

fn checked(path: &Path) -> Result<(String, StressLane), QaError> {
    let invalid = |detail: &str| QaError::ManifestInvalid {
        file: path.to_path_buf(),
        detail: detail.to_owned(),
    };
    let name = lane_name(path).ok_or_else(|| invalid("not a lane name"))?;
    let bytes = std::fs::read(path).map_err(|source| QaError::Io {
        operation: "read a lane manifest",
        source,
    })?;
    let lane: StressLane = serde_json::from_slice(&bytes).map_err(|source| QaError::Json {
        what: path.display().to_string(),
        source,
    })?;
    if lane.format != LANE_FORMAT {
        return Err(invalid("unknown format marker"));
    }
    if lane.variants.is_empty() || lane.timeout_minutes == 0 {
        return Err(invalid("a lane needs a runner and a non-zero timeout"));
    }
    if !path.with_extension("sh").is_file() {
        return Err(invalid("the lane script <lane>.sh is missing"));
    }
    Ok((name, lane))
}

/// Every lane in `dir`, by name.
///
/// # Errors
///
/// The directory is unreadable, or a manifest is invalid or has no script.
pub fn lanes(dir: &Path) -> Result<BTreeMap<String, StressLane>, QaError> {
    let entries = std::fs::read_dir(dir).map_err(|source| QaError::Io {
        operation: "list the stress lanes",
        source,
    })?;
    let mut manifests: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|source| QaError::Io {
                operation: "list the stress lanes",
                source,
            })?
            .path();
        if path.extension().is_some_and(|ext| ext == "json") {
            manifests.push(path);
        }
    }
    manifests.into_iter().map(|path| checked(&path)).collect()
}

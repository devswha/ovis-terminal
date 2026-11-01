//! Files under an audited evidence tree that are known not to be runner evidence. Each
//! exemption is tested on every run: it must name a file the audit really rejects.

use std::path::Path;

use serde::Deserialize;

use super::error::QualityError;
use super::report::Finding;

const FORMAT: &str = "ovis-evidence-exemptions/1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Exemptions {
    format: String,
    entries: Vec<Exemption>,
}

/// One exempt file, relative to the repository, and why it is not evidence.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Exemption {
    pub(super) path: String,
    pub(super) reason: String,
}

/// Reads the exemption list; an entry without a reason is a ledger error.
pub(super) fn load(path: &Path) -> Result<Vec<Exemption>, QualityError> {
    let text = std::fs::read_to_string(path).map_err(QualityError::io(path))?;
    let parsed: Exemptions = serde_json::from_str(&text).map_err(|source| QualityError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    let unexplained = parsed
        .entries
        .iter()
        .any(|entry| entry.reason.trim().is_empty());
    if parsed.format != FORMAT || unexplained {
        return Err(QualityError::Ledger {
            path: path.to_path_buf(),
            missing: format!("format {FORMAT} with a reason for every entry"),
        });
    }
    Ok(parsed.entries)
}

/// An exemption whose file was not audited, or that the audit accepts, is stale.
pub(super) fn stale(entries: &[Exemption], rejected: &[String]) -> Vec<Finding> {
    entries
        .iter()
        .filter(|entry| !rejected.contains(&entry.path))
        .map(|entry| {
            Finding::new(
                "stale-exemption",
                entry.path.clone(),
                "the file is not audited or is valid evidence; remove the exemption",
            )
        })
        .collect()
}

//! Ledger of amendments made to the released baseline before release.
//!
//! The ledger states the digest of the baseline as first released on a
//! `- released: sha256:<hex>` line before its first `## ` heading. Each `## `
//! entry then records one change with `- key: value` lines for every key of
//! [`ENTRY_KEYS`]; its `previous` digest continues the chain and its `current`
//! digest becomes the chain's head. The released baseline must hash to that head,
//! so any change to it without a new entry is refused.

use std::collections::BTreeMap;

use serde_json::Value;
use sha2::{Digest as _, Sha256};

use super::error::SchemaError;

/// Keys every amendment entry states.
pub const ENTRY_KEYS: [&str; 6] = ["date", "pr", "fields", "reason", "previous", "current"];

const DIGEST_PREFIX: &str = "sha256:";

/// Hex SHA-256 of the compact JSON encoding of `released`.
///
/// The digest covers the JSON content, not its formatting or line endings.
///
/// # Errors
///
/// Returns [`SchemaError::Json`] if `released` cannot be encoded.
pub fn baseline_digest(released: &Value) -> Result<String, SchemaError> {
    let encoded = serde_json::to_vec(released)?;
    let nibbles = Sha256::digest(&encoded)
        .into_iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f]);
    Ok(nibbles
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect())
}

/// One `## ` entry: its heading and its `- key: value` lines.
struct Entry<'a> {
    title: &'a str,
    values: BTreeMap<&'a str, &'a str>,
}

impl<'a> Entry<'a> {
    fn value(&self, key: &str) -> Result<&'a str, SchemaError> {
        self.values
            .get(key)
            .copied()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| malformed(&format!("{}.{key}", self.title)))
    }
}

fn malformed(location: &str) -> SchemaError {
    SchemaError::AmendmentsMalformed {
        location: location.to_owned(),
    }
}

fn digest_of<'a>(value: &'a str, location: &str) -> Result<&'a str, SchemaError> {
    value
        .strip_prefix(DIGEST_PREFIX)
        .ok_or_else(|| malformed(location))
}

/// The `released` digest line and the entries of `ledger`, in order.
fn parse(ledger: &str) -> (Option<&str>, Vec<Entry<'_>>) {
    let mut released = None;
    let mut entries: Vec<Entry<'_>> = Vec::new();
    for line in ledger.lines().map(str::trim_end) {
        if let Some(title) = line.strip_prefix("## ") {
            entries.push(Entry {
                title: title.trim(),
                values: BTreeMap::new(),
            });
            continue;
        }
        let Some((key, value)) = line
            .strip_prefix("- ")
            .and_then(|item| item.split_once(':'))
        else {
            continue;
        };
        let value = value.trim();
        match entries.last_mut() {
            None if key == "released" => released = Some(value),
            Some(entry) if ENTRY_KEYS.contains(&key) => {
                entry.values.insert(key, value);
            }
            None | Some(_) => {}
        }
    }
    (released, entries)
}

/// Fails unless `ledger` records every change that led to `released`.
///
/// # Errors
///
/// Returns [`SchemaError::AmendmentsMalformed`] when the released digest or an
/// entry key is missing or an entry's `previous` digest breaks the chain, and
/// [`SchemaError::AmendmentMissing`] when `released` hashes to no recorded digest.
pub fn check_amendments(released: &Value, ledger: &str) -> Result<(), SchemaError> {
    let (first, entries) = parse(ledger);
    let mut head = digest_of(first.ok_or_else(|| malformed("released"))?, "released")?;
    for entry in &entries {
        for key in ENTRY_KEYS {
            entry.value(key)?;
        }
        let location = format!("{}.previous", entry.title);
        if digest_of(entry.value("previous")?, &location)? != head {
            return Err(malformed(&location));
        }
        head = digest_of(entry.value("current")?, &format!("{}.current", entry.title))?;
    }
    let digest = baseline_digest(released)?;
    if digest == head {
        Ok(())
    } else {
        Err(SchemaError::AmendmentMissing {
            digest,
            previous: head.to_owned(),
        })
    }
}

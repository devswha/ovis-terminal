//! Reading pinned files and their SHA-256.

use std::path::Path;

use serde::de::DeserializeOwned;
use sha2::{Digest as _, Sha256};

use crate::error::ReplayError;

/// Lowercase hex SHA-256 of `bytes`.
pub(crate) fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect()
}

/// The bytes of `path`.
pub(crate) fn read(path: &Path) -> Result<Vec<u8>, ReplayError> {
    std::fs::read(path).map_err(|source| ReplayError::Io {
        operation: "read",
        path: path.to_path_buf(),
        source,
    })
}

/// `path` parsed as `T`.
pub(crate) fn json<T: DeserializeOwned>(path: &Path) -> Result<T, ReplayError> {
    serde_json::from_slice(&read(path)?).map_err(|source| ReplayError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Refuses a file whose declared format is not `expected`.
pub(crate) fn format(
    path: &Path,
    (expected, found): (&'static str, &str),
) -> Result<(), ReplayError> {
    if found == expected {
        return Ok(());
    }
    Err(ReplayError::Format {
        path: path.to_path_buf(),
        expected,
        found: found.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::sha256;

    #[test]
    fn digest_matches_the_published_vector() {
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}

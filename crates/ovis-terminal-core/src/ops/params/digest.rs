//! Params digests always carry the name of the algorithm that produced them.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// The algorithm a params digest was computed with; always stored beside the digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DigestAlgorithm {
    /// SHA-256 over the canonical JSON encoding.
    #[serde(rename = "sha-256")]
    Sha256,
}

/// Digest of the canonical params encoding, persisted as `{algorithm, hex}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "StoredDigest", into = "StoredDigest")]
pub struct ParamsDigest {
    algorithm: DigestAlgorithm,
    bytes: [u8; 32],
}

impl ParamsDigest {
    pub(in crate::ops) fn sha256(canonical: &[u8]) -> Self {
        Self {
            algorithm: DigestAlgorithm::Sha256,
            bytes: Sha256::digest(canonical).into(),
        }
    }

    /// A pre-78 row's bare digest, which was always SHA-256.
    pub(super) fn legacy(bytes: Vec<u8>) -> Result<Self, DigestError> {
        Ok(Self {
            algorithm: DigestAlgorithm::Sha256,
            bytes: bytes.try_into().map_err(|_| DigestError::Malformed)?,
        })
    }

    /// The algorithm that produced this digest.
    pub const fn algorithm(&self) -> DigestAlgorithm {
        self.algorithm
    }

    /// The digest as 64 lowercase hex digits.
    pub fn hex(&self) -> String {
        self.bytes
            .iter()
            .flat_map(|byte| [byte / 16, byte % 16])
            .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
            .collect()
    }
}

/// A ledger row's params digest could not be read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DigestError {
    /// The row carries neither an algorithm-named digest nor the pre-78 SHA-256 bytes.
    #[error("the ledger row names no params digest")]
    Missing,
    /// The digest is not 32 bytes (64 lowercase hex digits).
    #[error("a sha-256 params digest is 32 bytes, written as 64 lowercase hex digits")]
    Malformed,
}

#[derive(Serialize, Deserialize)]
struct StoredDigest {
    algorithm: DigestAlgorithm,
    hex: String,
}

impl From<ParamsDigest> for StoredDigest {
    fn from(digest: ParamsDigest) -> Self {
        Self {
            algorithm: digest.algorithm,
            hex: digest.hex(),
        }
    }
}

impl TryFrom<StoredDigest> for ParamsDigest {
    type Error = DigestError;

    fn try_from(stored: StoredDigest) -> Result<Self, DigestError> {
        let (pairs, rest) = stored.hex.as_bytes().as_chunks::<2>();
        if pairs.len() != 32 || !rest.is_empty() {
            return Err(DigestError::Malformed);
        }
        let nibble = |digit: u8| match digit {
            b'0'..=b'9' | b'a'..=b'f' => char::from(digit).to_digit(16),
            _ => None,
        };
        let mut bytes = [0_u8; 32];
        for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
            let value = nibble(*high)
                .zip(nibble(*low))
                .and_then(|(high, low)| {
                    high.checked_mul(16)?
                        .checked_add(low)
                        .and_then(|v| u8::try_from(v).ok())
                })
                .ok_or(DigestError::Malformed)?;
            *byte = value;
        }
        Ok(Self {
            algorithm: stored.algorithm,
            bytes,
        })
    }
}

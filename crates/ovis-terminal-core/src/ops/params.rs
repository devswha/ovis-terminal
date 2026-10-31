//! Per-method params bounds, algorithm-named params digests and the inspection copy.
//!
//! Reuse of an operation identity is decided by the SHA-256 digest of the canonical params,
//! never by the stored copy: the copy keeps at most [`PARAMS_COPY_BYTES`] for inspection and
//! says whether it was truncated.

use std::fmt;

use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_protocol::schema::scalar::ByteCount;
use ovis_terminal_store::{CasWrite, Payload, ResourceLookup, Version};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::ledger::OperationKey;
use super::{OperationError, OperationEvent, OperationSnapshot, Operations};
use crate::events::{EventData, EventError, SnapshotProducer};
use crate::registry::RuntimeId;

mod digest;
#[cfg(test)]
mod tests;

pub use digest::{DigestAlgorithm, DigestError, ParamsDigest};

/// Canonical params bound of every method without an advertised payload bound.
pub const DEFAULT_PARAMS_BYTES: ByteCount = ByteCount::new(256 << 10);

/// Room beside an advertised payload for the method's other fields (name, version, machine).
pub const PARAMS_ENVELOPE_BYTES: ByteCount = ByteCount::new(4 << 10);

/// Longest prefix of the canonical params retained for inspection.
pub const PARAMS_COPY_BYTES: usize = 256 << 10;

/// The canonical-params byte bound one method admits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParamsBound {
    /// `doc.set`: the advertised `doc_max_bytes` plus [`PARAMS_ENVELOPE_BYTES`].
    Document,
    /// Every other method: [`DEFAULT_PARAMS_BYTES`].
    Default,
}

impl ParamsBound {
    /// The bound that applies to `method`.
    pub fn of(method: &str) -> Self {
        match method {
            "doc.set" => Self::Document,
            _ => Self::Default,
        }
    }

    /// Largest accepted canonical params encoding, in bytes.
    pub const fn bytes(self) -> ByteCount {
        match self {
            Self::Document => ByteCount::new(
                Limits::v1()
                    .doc_max_bytes
                    .get()
                    .saturating_add(PARAMS_ENVELOPE_BYTES.get()),
            ),
            Self::Default => DEFAULT_PARAMS_BYTES,
        }
    }

    /// Recovery guidance naming the bound.
    pub const fn hint(self) -> &'static str {
        match self {
            Self::Document => {
                "keep doc.set params within doc_max_bytes plus 4096 bytes for name, version and machine"
            }
            Self::Default => "keep canonical params within the 262144-byte default bound",
        }
    }
}

/// Canonical params as one ledger row holds them.
///
/// Identity is the digest alone: rows accepted before the inspection copy existed (no
/// extent) still match an identical retry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "StoredParams", into = "StoredParams")]
pub(super) struct Params {
    pub digest: ParamsDigest,
    pub extent: Option<Extent>,
}

impl PartialEq for Params {
    fn eq(&self, other: &Self) -> bool {
        self.digest == other.digest
    }
}

impl Eq for Params {}

impl Params {
    /// Deletes the inspection copy together with its evicted row.
    pub(super) fn evict(&self, key: &OperationKey, runtime: RuntimeId) -> Option<CasWrite> {
        // The copy is written once at acceptance, so its live version is always 1.
        self.extent
            .map(|_| CasWrite::new(key.params_resource(runtime), Version::new(1), None))
    }
}

/// Size of the canonical params and whether the inspection copy is a prefix of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Extent {
    pub bytes: ByteCount,
    pub truncated: bool,
}

/// Row fields of [`Params`]; `params` is the pre-78 bare SHA-256, read but never written.
#[derive(Serialize, Deserialize)]
struct StoredParams {
    #[serde(default, rename = "params", skip_serializing)]
    legacy: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    params_digest: Option<ParamsDigest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    params_copy: Option<Extent>,
}

impl From<Params> for StoredParams {
    fn from(params: Params) -> Self {
        Self {
            legacy: None,
            params_digest: Some(params.digest),
            params_copy: params.extent,
        }
    }
}

impl TryFrom<StoredParams> for Params {
    type Error = DigestError;

    fn try_from(stored: StoredParams) -> Result<Self, DigestError> {
        let digest = match (stored.params_digest, stored.legacy) {
            (Some(digest), _) => digest,
            (None, Some(legacy)) => ParamsDigest::legacy(legacy)?,
            (None, None) => return Err(DigestError::Missing),
        };
        Ok(Self {
            digest,
            extent: stored.params_copy,
        })
    }
}

/// The inspection prefix of one request's canonical params, written beside its row.
pub(super) struct CopyBytes(Box<[u8]>);

impl CopyBytes {
    /// Keeps at most [`PARAMS_COPY_BYTES`], cut at a UTF-8 character boundary.
    pub(super) fn retain(mut canonical: Vec<u8>) -> (Self, Extent) {
        let bytes = ByteCount::new(u64::try_from(canonical.len()).unwrap_or(u64::MAX));
        let truncated = canonical.len() > PARAMS_COPY_BYTES;
        if truncated {
            // UTF-8 continuation bytes are 0b10xx_xxxx; back up to a character start.
            let mut end = PARAMS_COPY_BYTES;
            while canonical.get(end).is_some_and(|byte| byte & 0xc0 == 0x80) {
                end = end.saturating_sub(1);
            }
            canonical.truncate(end);
        }
        (
            Self(canonical.into_boxed_slice()),
            Extent { bytes, truncated },
        )
    }

    pub(super) fn write(&self, key: &OperationKey, runtime: RuntimeId) -> CasWrite {
        CasWrite::new(
            key.params_resource(runtime),
            Version::ABSENT,
            Some(Payload::from(self.0.to_vec())),
        )
    }
}

impl fmt::Debug for CopyBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CopyBytes")
            .field("len", &self.0.len())
            .finish()
    }
}

/// What the ledger retains of an operation's canonical params.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ParamsView {
    /// The digest that decides reuse of the operation identity.
    pub digest: ParamsDigest,
    /// The inspection copy; `None` for operations accepted before copies were retained.
    pub copy: Option<ParamsCopy>,
}

/// A stored prefix of the canonical params encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ParamsCopy {
    /// At most [`PARAMS_COPY_BYTES`] of the canonical encoding.
    pub prefix: Box<[u8]>,
    /// Length of the whole canonical encoding.
    pub canonical_bytes: ByteCount,
    /// Whether `prefix` is shorter than the canonical encoding.
    pub truncated: bool,
}

impl<D, P> Operations<'_, D, P>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned,
    P: SnapshotProducer,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    /// The retained digest and inspection copy of this principal's operation params.
    ///
    /// # Errors
    ///
    /// Returns `not_found`, `op_expired`, or `operation_store_invalid` when a live row has
    /// lost its copy.
    pub fn params(&self, op: &OpId) -> Result<ParamsView, OperationError> {
        let row = self.current(op)?;
        let Params { digest, extent } = row.entry.params;
        let Some(extent) = extent else {
            return Ok(ParamsView { digest, copy: None });
        };
        let runtime = self.context.identity.runtime_id();
        let prefix = self.coordinator.read_view(|snapshot| {
            let copy = snapshot
                .resource(&row.key.params_resource(runtime))
                .map_err(EventError::from)?;
            match copy {
                ResourceLookup::Live { value, .. } => Ok(Box::from(value.as_bytes())),
                _ => match snapshot
                    .resource(&row.key.resource(runtime))
                    .map_err(EventError::from)?
                {
                    // The row and its copy are evicted in one commit after current() read.
                    ResourceLookup::Live { .. } => Err(OperationError::InvalidLedger),
                    _ => Err(OperationError::Expired),
                },
            }
        })?;
        Ok(ParamsView {
            digest,
            copy: Some(ParamsCopy {
                prefix,
                canonical_bytes: extent.bytes,
                truncated: extent.truncated,
            }),
        })
    }
}

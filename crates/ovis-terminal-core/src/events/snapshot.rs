//! Composable typed sections and revision-pinned snapshot pages.

use ovis_terminal_protocol::rpc::DecimalU64;
use ovis_terminal_protocol::schema::ids::Digest as DigestValue;
use ovis_terminal_protocol::schema::scalar::{ByteCount, ByteOffset};
use ovis_terminal_store::Snapshot;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::{Cursor, EventError, Filter};

/// Completeness is advertised only by an implemented section producer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "availability", content = "records", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Section<T> {
    /// The owning producer has not supplied this section.
    Unavailable,
    /// The producer captured every record in this section at the snapshot cursor.
    Complete(T),
}

/// Typed composition point for current layout/terminal and later domain producers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SnapshotSections<L, T, A = (), S = ()> {
    /// Structural layout, including workspace, tab and pane identities.
    pub layout: Section<L>,
    /// Terminal generations, geometry and restoration records.
    pub terminals: Section<T>,
    /// Agent/session records, unavailable until their producer exists.
    pub agents: Section<A>,
    /// Service records, unavailable until their producer exists.
    pub services: Section<S>,
}

impl<L, T, A, S> SnapshotSections<L, T, A, S> {
    /// Installs the owning agent producer's typed section at this same revision.
    pub fn with_agents<N>(self, agents: Section<N>) -> SnapshotSections<L, T, N, S> {
        SnapshotSections {
            layout: self.layout,
            terminals: self.terminals,
            agents,
            services: self.services,
        }
    }

    /// Installs the owning service producer's typed section at this same revision.
    pub fn with_services<N>(self, services: Section<N>) -> SnapshotSections<L, T, A, N> {
        SnapshotSections {
            layout: self.layout,
            terminals: self.terminals,
            agents: self.agents,
            services,
        }
    }
}

impl<L, T> SnapshotSections<L, T> {
    /// Composes implemented layout and terminal producers without inventing later records.
    pub const fn new(layout: Section<L>, terminals: Section<T>) -> Self {
        Self {
            layout,
            terminals,
            agents: Section::Unavailable,
            services: Section::Unavailable,
        }
    }
}

/// Captures owned typed sections using only the supplied, revision-pinned WAL view.
///
/// Producers must read all authoritative records through `snapshot`, never through
/// independent live actor reads. Assembly supplies the actual domain producers;
/// the coordinator does not advertise an empty implementation of any domain.
pub trait SnapshotProducer {
    /// Composable typed snapshot record produced at one durable revision.
    type Sections: Serialize;

    /// Captures every implemented section from the same read transaction.
    ///
    /// # Errors
    ///
    /// Returns the typed store/decoding failure, never an empty successful section.
    fn capture(&self, snapshot: &Snapshot, filter: &Filter) -> Result<Self::Sections, EventError>;
}

/// Complete implemented state and the cursor captured at the same barrier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct StateSnapshot<S> {
    /// Revision shared with subsequent events.
    pub cursor: Cursor,
    /// Availability and typed records of each section.
    pub sections: S,
}

/// Opaque page lifetime identity; a new core never accepts an old core's token.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SnapshotToken(String);

impl SnapshotToken {
    pub(super) fn new() -> Self {
        Self(uuid::Uuid::now_v7().to_string())
    }
}

/// Zero-based position of a page within one pinned snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PageIndex(DecimalU64);

impl PageIndex {
    /// Wraps a zero-based page position.
    pub const fn new(value: u64) -> Self {
        Self(DecimalU64::new(value))
    }

    /// Returns the native page position.
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    pub(super) fn checked_next(self) -> Result<Self, EventError> {
        self.get()
            .checked_add(1)
            .map(Self::new)
            .ok_or(EventError::LimitExceeded)
    }
}

/// Number of pages in one complete pinned snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PageCount(DecimalU64);

impl PageCount {
    /// Wraps a nonzero page count.
    pub const fn new(value: u64) -> Self {
        Self(DecimalU64::new(value))
    }

    /// Returns the native page count.
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    pub(super) fn for_bytes(bytes: usize, page_bytes: usize) -> Result<Self, EventError> {
        let pages = bytes.div_ceil(page_bytes).max(1);
        Ok(Self::new(
            u64::try_from(pages).map_err(|_| EventError::LimitExceeded)?,
        ))
    }
}

/// SHA-256 digest of the complete serialized snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SnapshotDigest(DigestValue);

impl SnapshotDigest {
    pub(super) fn for_bytes(bytes: &[u8]) -> Self {
        Self(DigestValue::new(format!(
            "sha256:{:x}",
            Sha256::digest(bytes)
        )))
    }

    /// Borrows the algorithm-prefixed hexadecimal digest.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// A snapshot lease retained until explicit release or core shutdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SnapshotLease {
    /// Token required for every page and release.
    pub token: SnapshotToken,
    /// Constant revision across the entire lease.
    pub cursor: Cursor,
    /// Exact serialized length of the complete snapshot.
    pub total: ByteCount,
    /// Number of contiguous pages required before reconstruction.
    pub page_count: PageCount,
    /// End-to-end digest of the complete serialized representation.
    pub digest: SnapshotDigest,
}

/// A bounded chunk of one complete serialized typed snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SnapshotPage {
    /// Exact pinned snapshot this page belongs to.
    pub token: SnapshotToken,
    /// Cursor remains constant even if mutations commit between page requests.
    pub cursor: Cursor,
    /// Exact byte count of the full snapshot.
    pub total: ByteCount,
    /// Zero-based position in the complete page sequence.
    pub page_index: PageIndex,
    /// Number of pages required for complete reconstruction.
    pub page_count: PageCount,
    /// End-to-end digest shared by every page of this snapshot.
    pub digest: SnapshotDigest,
    /// Start of these bytes in the serialized representation.
    pub offset: ByteOffset,
    /// Continuation, absent only on the final page.
    pub next_offset: Option<ByteOffset>,
    /// Bytes concatenated by the caller before deserializing the snapshot.
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct Pinned {
    // Keep the WAL read transaction alive for the full page lease.
    pub _view: Snapshot,
    pub token: SnapshotToken,
    pub cursor: Cursor,
    pub bytes: Vec<u8>,
    pub page_count: PageCount,
    pub digest: SnapshotDigest,
}

impl Pinned {
    pub(super) fn page(
        &self,
        offset: ByteOffset,
        limit: usize,
    ) -> Result<SnapshotPage, EventError> {
        let start = usize::try_from(offset.get()).map_err(|_| EventError::InvalidOffset)?;
        if start >= self.bytes.len() || !start.is_multiple_of(limit) {
            return Err(EventError::InvalidOffset);
        }
        let end = start.saturating_add(limit).min(self.bytes.len());
        let data = self
            .bytes
            .get(start..end)
            .ok_or(EventError::InvalidOffset)?
            .to_vec();
        let index = start.checked_div(limit).ok_or(EventError::LimitExceeded)?;
        let page_index =
            PageIndex::new(u64::try_from(index).map_err(|_| EventError::LimitExceeded)?);
        Ok(SnapshotPage {
            token: self.token.clone(),
            cursor: self.cursor,
            total: ByteCount::new(
                u64::try_from(self.bytes.len()).map_err(|_| EventError::LimitExceeded)?,
            ),
            page_index,
            page_count: self.page_count,
            digest: self.digest.clone(),
            offset,
            next_offset: if end < self.bytes.len() {
                Some(ByteOffset::new(
                    u64::try_from(end).map_err(|_| EventError::LimitExceeded)?,
                ))
            } else {
                None
            },
            data,
        })
    }
}

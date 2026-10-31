//! Checked reconstruction of digest-bound snapshot page sequences.

use serde::de::DeserializeOwned;

use super::{EventError, PageIndex, SnapshotDigest, SnapshotLease, SnapshotPage, StateSnapshot};

/// Why a page sequence could not produce complete state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotTornReason {
    /// A later page arrived before the required index.
    MissingPage,
    /// An already consumed page arrived again.
    DuplicatePage,
    /// Page metadata disagreed with the pinned snapshot lease.
    MetadataMismatch,
    /// A page carried fewer or more bytes than its declared range.
    TruncatedPage,
    /// The complete bytes did not match the pinned end-to-end digest.
    DigestMismatch,
    /// Digest-valid bytes did not decode as the requested snapshot type.
    InvalidSnapshot,
}

/// Incremental validator that exposes state only after every page and digest match.
#[derive(Debug)]
pub struct SnapshotAssembler {
    lease: SnapshotLease,
    next_page: PageIndex,
    next_offset: u64,
    bytes: Vec<u8>,
}

impl SnapshotAssembler {
    /// Starts reconstruction from trusted lease metadata returned at subscription startup.
    pub const fn new(lease: SnapshotLease) -> Self {
        Self {
            lease,
            next_page: PageIndex::new(0),
            next_offset: 0,
            bytes: Vec::new(),
        }
    }

    /// Validates and appends exactly the next page.
    ///
    /// # Errors
    ///
    /// Returns `snapshot_torn` before retaining bytes from a missing, duplicated,
    /// reordered, truncated or foreign page.
    pub fn push(&mut self, page: &SnapshotPage) -> Result<(), EventError> {
        self.validate_metadata(page)?;
        if page.page_index != self.next_page {
            let reason = if page.page_index < self.next_page {
                SnapshotTornReason::DuplicatePage
            } else {
                SnapshotTornReason::MissingPage
            };
            return Err(self.torn(Some(page.page_index), reason));
        }
        if page.offset.get() != self.next_offset {
            return Err(self.torn(Some(page.page_index), SnapshotTornReason::TruncatedPage));
        }
        let data_len = u64::try_from(page.data.len()).map_err(|_| EventError::LimitExceeded)?;
        let end = self
            .next_offset
            .checked_add(data_len)
            .ok_or(EventError::LimitExceeded)?;
        let final_page = page.page_index.checked_next()?.get() == page.page_count.get();
        match (final_page, page.next_offset) {
            (true, None) if end == page.total.get() => {}
            (false, Some(next)) if next.get() == end && end < page.total.get() => {}
            _ => {
                return Err(self.torn(Some(page.page_index), SnapshotTornReason::TruncatedPage));
            }
        }
        self.bytes.extend_from_slice(&page.data);
        self.next_page = self.next_page.checked_next()?;
        self.next_offset = end;
        Ok(())
    }

    /// Verifies count, length and SHA-256 before decoding complete typed state.
    ///
    /// # Errors
    ///
    /// Returns `snapshot_torn` for any incomplete or corrupted sequence.
    pub fn finish<S: DeserializeOwned>(self) -> Result<StateSnapshot<S>, EventError> {
        if self.next_page.get() != self.lease.page_count.get() {
            return Err(self.torn(None, SnapshotTornReason::MissingPage));
        }
        if self.next_offset != self.lease.total.get() {
            return Err(self.torn(None, SnapshotTornReason::TruncatedPage));
        }
        if SnapshotDigest::for_bytes(&self.bytes) != self.lease.digest {
            return Err(self.torn(None, SnapshotTornReason::DigestMismatch));
        }
        let final_page = PageIndex::new(self.lease.page_count.get().saturating_sub(1));
        serde_json::from_slice(&self.bytes).map_err(|source| EventError::SnapshotDecode {
            page: final_page,
            source,
        })
    }

    fn validate_metadata(&self, page: &SnapshotPage) -> Result<(), EventError> {
        if page.token != self.lease.token
            || page.cursor != self.lease.cursor
            || page.total != self.lease.total
            || page.page_count != self.lease.page_count
            || page.digest != self.lease.digest
            || page.page_count.get() == 0
            || page.page_index.get() >= page.page_count.get()
        {
            return Err(self.torn(Some(page.page_index), SnapshotTornReason::MetadataMismatch));
        }
        Ok(())
    }

    const fn torn(
        &self,
        received_page: Option<PageIndex>,
        reason: SnapshotTornReason,
    ) -> EventError {
        EventError::SnapshotTorn {
            expected_page: self.next_page,
            received_page,
            reason,
        }
    }
}

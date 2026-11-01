//! Bounded pages of one pinned snapshot revision with an explicit lease expiry.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ovis_terminal_protocol::schema::scalar::{ByteCount, ByteOffset, TimestampMs};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::CommitError;
use crate::events::{
    Coordinator, Cursor, EventData, EventError, PageCount, SnapshotDigest, SnapshotProducer,
    SnapshotToken,
};

/// How long a pinned snapshot stays readable after it is opened, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageTtl(u64);

impl PageTtl {
    /// A nonzero lease length.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` for zero.
    pub const fn new(millis: u64) -> Result<Self, CommitError> {
        if millis == 0 {
            return Err(CommitError::InvalidBound { bound: "page ttl" });
        }
        Ok(Self(millis))
    }
}

/// A pinned revision: every page of it carries the same cursor, total and expiry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Window {
    /// Token every page and the release name.
    pub token: SnapshotToken,
    /// The one revision all pages are pinned to.
    pub cursor: Cursor,
    /// Exact serialized length of the complete snapshot.
    pub total: ByteCount,
    /// Pages of the server's page bound needed to read it.
    pub page_count: PageCount,
    /// End-to-end digest of the complete snapshot.
    pub digest: SnapshotDigest,
    /// After this instant no page is served and the pin is released.
    pub expires_ms: TimestampMs,
}

/// One bounded page of a [`Window`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WindowPage {
    /// The window's revision, unchanged by commits since it was pinned.
    pub cursor: Cursor,
    /// Exact serialized length of the complete snapshot.
    pub total: ByteCount,
    /// Start of `data` in the complete snapshot.
    pub offset: ByteOffset,
    /// Where the next page starts; absent after the last byte.
    pub continuation: Option<ByteOffset>,
    /// The server's page bound delivered fewer bytes than were asked for.
    pub truncated: bool,
    /// The window's expiry.
    pub expires_ms: TimestampMs,
    /// The bytes.
    pub data: Vec<u8>,
}

/// Opens, pages, expires and releases pinned snapshot windows; time is supplied.
#[derive(Debug)]
pub struct Pager<D, P> {
    core: Arc<Coordinator<D, P>>,
    ttl: PageTtl,
    leases: Mutex<BTreeMap<SnapshotToken, TimestampMs>>,
}

impl<D, P> Pager<D, P>
where
    D: EventData + Clone + Serialize + DeserializeOwned,
    P: SnapshotProducer,
{
    /// Pages snapshots of `core`, each readable for `ttl` after it is opened.
    pub const fn new(core: Arc<Coordinator<D, P>>, ttl: PageTtl) -> Self {
        Self {
            core,
            ttl,
            leases: Mutex::new(BTreeMap::new()),
        }
    }

    /// Releases expired windows, then pins the current revision until `now + ttl`.
    ///
    /// # Errors
    ///
    /// Returns `limit_exceeded` when every pin slot is held, or a typed capture failure.
    pub fn open(&self, now: TimestampMs) -> Result<Window, CommitError> {
        self.reap(now)?;
        let expires_ms = now
            .get()
            .checked_add(self.ttl.0)
            .map(TimestampMs::new)
            .ok_or(CommitError::InvalidBound {
                bound: "page expiry",
            })?;
        let lease = self.core.pin_snapshot()?;
        self.lock()?.insert(lease.token.clone(), expires_ms);
        Ok(Window {
            token: lease.token,
            cursor: lease.cursor,
            total: lease.total,
            page_count: lease.page_count,
            digest: lease.digest,
            expires_ms,
        })
    }

    /// Reads up to `limit` bytes at `offset` at time `now`; at or after expiry the pin is
    /// released.
    ///
    /// # Errors
    ///
    /// Returns `page_expired`, `stale_snapshot` for an unknown or released token, or
    /// `invalid_params` for an offset past the end or a zero limit.
    pub fn page(
        &self,
        token: &SnapshotToken,
        (offset, limit, now): (ByteOffset, usize, TimestampMs),
    ) -> Result<WindowPage, CommitError> {
        let expires_ms = self.live(token, now)?;
        if limit == 0 {
            return Err(CommitError::InvalidBound {
                bound: "page limit",
            });
        }
        let page_bytes = self.core.limits().page_bytes;
        let start = usize::try_from(offset.get()).map_err(|_| EventError::InvalidOffset)?;
        let aligned = start
            .checked_div(page_bytes)
            .unwrap_or(0)
            .saturating_mul(page_bytes);
        let base = u64::try_from(aligned).map_err(|_| EventError::InvalidOffset)?;
        let page = self.core.page(token, ByteOffset::new(base))?;
        let total = usize::try_from(page.total.get()).map_err(|_| EventError::InvalidOffset)?;
        if start >= total {
            return Err(EventError::InvalidOffset.into());
        }
        let skip = start.saturating_sub(aligned);
        let available = page.data.get(skip..).ok_or(EventError::InvalidOffset)?;
        let data = available
            .get(..limit.min(available.len()))
            .unwrap_or(available)
            .to_vec();
        let end = start.saturating_add(data.len());
        let continuation = (end < total)
            .then(|| u64::try_from(end).map(ByteOffset::new))
            .transpose()
            .map_err(|_| EventError::InvalidOffset)?;
        Ok(WindowPage {
            cursor: page.cursor,
            total: page.total,
            offset,
            truncated: continuation.is_some() && data.len() < limit,
            continuation,
            expires_ms,
            data,
        })
    }

    /// Releases a window before its expiry.
    ///
    /// # Errors
    ///
    /// Returns `stale_snapshot` for an unknown or already released token.
    pub fn release(&self, token: &SnapshotToken) -> Result<(), CommitError> {
        self.lock()?.remove(token);
        Ok(self.core.release_snapshot(token)?)
    }

    /// Releases every window whose expiry is at or before `now`; returns how many.
    ///
    /// # Errors
    ///
    /// Returns `coordinator_stopped` after poisoning.
    pub fn reap(&self, now: TimestampMs) -> Result<usize, CommitError> {
        let expired: Vec<SnapshotToken> = {
            let mut leases = self.lock()?;
            let expired = leases
                .iter()
                .filter(|(_, expires)| **expires <= now)
                .map(|(token, _)| token.clone())
                .collect::<Vec<_>>();
            for token in &expired {
                leases.remove(token);
            }
            expired
        };
        for token in &expired {
            self.core.release_snapshot(token)?;
        }
        Ok(expired.len())
    }

    fn live(&self, token: &SnapshotToken, now: TimestampMs) -> Result<TimestampMs, CommitError> {
        let expires_ms = *self
            .lock()?
            .get(token)
            .ok_or(CommitError::Event(EventError::StaleSnapshot))?;
        if now < expires_ms {
            return Ok(expires_ms);
        }
        self.release(token)?;
        Err(CommitError::PageExpired {
            expired_ms: expires_ms,
        })
    }

    fn lock(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeMap<SnapshotToken, TimestampMs>>, CommitError> {
        self.leases
            .lock()
            .map_err(|_| CommitError::Event(EventError::Poisoned))
    }
}

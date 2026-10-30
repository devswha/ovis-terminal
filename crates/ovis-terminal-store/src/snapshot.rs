//! Read transactions pinned to one revision of the store.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension as _, params};

use crate::error::StoreError;
use crate::evidence::RecoveryEvidence;
use crate::ids::{EventKind, OpId, Payload, ResourceKey, Revision, Version};

const READER_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The state of one resource at a snapshot's revision.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourceLookup {
    /// The resource holds a value.
    Live {
        /// Its compare-and-swap version.
        version: Version,
        /// Its value.
        value: Payload,
        /// The revision that wrote it.
        revision: Revision,
    },
    /// The resource was deleted; its version continues from here.
    Tombstone {
        /// Its compare-and-swap version.
        version: Version,
        /// The revision that deleted it.
        revision: Revision,
    },
    /// The resource was never written.
    Absent,
}

/// The receipt of one operation id at a snapshot's revision.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReceiptLookup {
    /// The operation committed.
    Found {
        /// The revision it committed at.
        revision: Revision,
        /// The result stored with it.
        result: Payload,
    },
    /// Receipts below `floor` were evicted, so this id cannot be told apart from an expired one.
    NotRetained {
        /// The lowest revision whose receipts are still retained.
        floor: Revision,
    },
    /// No receipt was ever evicted and none exists for this id.
    Absent,
}

/// One committed event.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoredEvent {
    /// Its sequence number, shared with store revisions.
    pub seq: Revision,
    /// Its type.
    pub kind: EventKind,
    /// Its body.
    pub data: Payload,
    /// The operation that committed it.
    pub op: OpId,
}

/// A page of events after a cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EventPage {
    /// Every event after the cursor up to the page limit, in sequence order.
    Events(Vec<StoredEvent>),
    /// Events right after the cursor were evicted; `floor` is the oldest retained sequence number.
    Expired {
        /// The oldest retained sequence number.
        floor: Revision,
    },
}

/// A read transaction that sees the store exactly as of [`Snapshot::revision`].
#[derive(Debug)]
pub struct Snapshot {
    connection: Connection,
    revision: Revision,
}

impl Snapshot {
    pub(crate) fn begin(db: &Path) -> Result<Self, StoreError> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = Connection::open_with_flags(db, flags)?;
        connection.pragma_update(None, "query_only", true)?;
        connection.busy_timeout(READER_BUSY_TIMEOUT)?;
        connection.execute_batch("BEGIN DEFERRED")?;
        // The first read inside the transaction fixes its WAL snapshot.
        let revision = connection.query_row("SELECT revision FROM head", [], |row| row.get(0))?;
        Ok(Self {
            connection,
            revision,
        })
    }

    /// The revision every read through this snapshot observes.
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Looks up one resource.
    ///
    /// # Errors
    ///
    /// Returns `persistence_failed` when SQLite cannot read the row.
    pub fn resource(&self, key: &ResourceKey) -> Result<ResourceLookup, StoreError> {
        let row: Option<(Version, Option<Vec<u8>>, Revision)> = self
            .connection
            .query_row(
                "SELECT version, value, revision FROM resources WHERE kind = ?1 AND id = ?2",
                [key.kind(), key.id()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        Ok(match row {
            None => ResourceLookup::Absent,
            Some((version, None, revision)) => ResourceLookup::Tombstone { version, revision },
            Some((version, Some(bytes), revision)) => ResourceLookup::Live {
                version,
                value: Payload::from(bytes),
                revision,
            },
        })
    }

    /// Looks up the receipt of `op`.
    ///
    /// # Errors
    ///
    /// Returns `persistence_failed` when SQLite cannot read the receipt or its floor.
    pub fn receipt(&self, op: &OpId) -> Result<ReceiptLookup, StoreError> {
        let row: Option<(Revision, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT revision, result FROM receipts WHERE op = ?1",
                [op.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((revision, result)) = row {
            return Ok(ReceiptLookup::Found {
                revision,
                result: Payload::from(result),
            });
        }
        let floor = self.floor("receipts")?;
        Ok(if floor > Revision::ZERO {
            ReceiptLookup::NotRetained { floor }
        } else {
            ReceiptLookup::Absent
        })
    }

    /// Up to `limit` events with a sequence number above `after`.
    ///
    /// # Errors
    ///
    /// Returns `persistence_failed` when SQLite cannot read the events or their floor.
    pub fn events_after(&self, after: Revision, limit: usize) -> Result<EventPage, StoreError> {
        let floor = self.floor("events")?;
        if after.checked_add(1).is_some_and(|next| next < floor) {
            return Ok(EventPage::Expired { floor });
        }
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut statement = self.connection.prepare(
            "SELECT seq, kind, data, op FROM events WHERE seq > ?1 ORDER BY seq LIMIT ?2",
        )?;
        let events = statement
            .query_map(params![after, limit], |row| {
                Ok(StoredEvent {
                    seq: row.get(0)?,
                    kind: EventKind::new(row.get::<_, String>(1)?),
                    data: Payload::from(row.get::<_, Vec<u8>>(2)?),
                    op: OpId::new(row.get::<_, String>(3)?),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(EventPage::Events(events))
    }

    /// Recovery evidence carried into this database by a rebuild; reading never clears it.
    ///
    /// # Errors
    ///
    /// Returns `persistence_failed` when a stored record cannot be read or decoded.
    pub fn recovery_evidence(&self) -> Result<Vec<RecoveryEvidence>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT evidence FROM recovery_evidence ORDER BY id")?;
        let texts = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        texts
            .iter()
            .map(|text| {
                serde_json::from_str(text).map_err(|source| StoreError::Evidence {
                    path: "state.db#recovery_evidence".into(),
                    source,
                })
            })
            .collect()
    }

    fn floor(&self, stream: &str) -> Result<Revision, StoreError> {
        Ok(self.connection.query_row(
            "SELECT floor FROM floors WHERE stream = ?1",
            [stream],
            |row| row.get(0),
        )?)
    }
}

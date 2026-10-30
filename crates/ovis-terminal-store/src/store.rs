//! Opening, rebuilding and closing a store root.

mod apply;
mod quarantine;
mod schema;
mod writer;

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

use self::quarantine::DB_FILE;
pub use self::quarantine::read_quarantine_evidence;
use self::schema::Probe;
pub use self::schema::SCHEMA_VERSION;
pub use self::writer::Ticket;
use self::writer::Writer;
use crate::error::StoreError;
use crate::evidence::{MaterialDisposition, RecoveryCause};
use crate::mutation::Mutation;
use crate::options::{StoreLimits, StoreOptions};
use crate::snapshot::Snapshot;

/// An opened store: one dedicated writer and any number of pinned read snapshots.
#[derive(Debug)]
pub struct Store {
    db: PathBuf,
    limits: StoreLimits,
    writer: Writer,
}

impl Store {
    /// Opens the store under `root`, creating it only when nothing was ever there.
    ///
    /// # Errors
    ///
    /// Returns `store_corrupted` after moving a damaged database and WAL into quarantine,
    /// `store_unsupported` after copying a newer-schema database (left in place), and the
    /// same error again on later opens until the store is rebuilt; `persistence_failed`
    /// when the root cannot be read or written.
    pub fn open(root: &Path, options: StoreOptions) -> Result<Self, StoreError> {
        let db = root.join(DB_FILE);
        match settle(root, &db)? {
            Some((connection, version)) => Self::start(db, connection, (version, options)),
            None => match quarantine::latest_incident(root)? {
                Some(evidence) => Err(quarantine::into_error(evidence)),
                None => Self::start(db.clone(), Connection::open(&db)?, (0, options)),
            },
        }
    }

    /// Starts an empty store after a quarantine moved the database away, carrying every
    /// recorded incident into it so the evidence stays queryable.
    ///
    /// # Errors
    ///
    /// Returns `nothing_to_rebuild` when a healthy database exists or nothing was quarantined,
    /// and the same errors as [`Store::open`] when the existing database is damaged or newer.
    pub fn rebuild_after_quarantine(
        root: &Path,
        options: StoreOptions,
    ) -> Result<Self, StoreError> {
        let db = root.join(DB_FILE);
        let incidents = quarantine::read_quarantine_evidence(root)?;
        if settle(root, &db)?.is_some() || incidents.is_empty() {
            return Err(StoreError::NothingToRebuild {
                root: root.to_path_buf(),
            });
        }
        let mut connection = Connection::open(&db)?;
        schema::prepare(&mut connection, 0)?;
        let tx = connection.transaction()?;
        for evidence in &incidents {
            let text = serde_json::to_string(evidence).map_err(|source| StoreError::Evidence {
                path: evidence.location.clone(),
                source,
            })?;
            tx.execute(
                "INSERT INTO recovery_evidence (id, evidence) VALUES (?1, ?2)",
                params![evidence.id.as_str(), text],
            )?;
        }
        tx.commit()?;
        Self::start(db, connection, (SCHEMA_VERSION, options))
    }

    /// Queues `mutation` for the writer.
    ///
    /// # Errors
    ///
    /// Returns `too_large` for an oversized payload and `backpressure` when the bounded queue
    /// is full, both before anything is queued; `writer_stopped` when the writer has exited.
    pub fn submit(&self, mutation: Mutation) -> Result<Ticket, StoreError> {
        let actual = mutation.largest_payload();
        if actual > self.limits.max_value_bytes {
            return Err(StoreError::TooLarge {
                limit: self.limits.max_value_bytes,
                actual,
            });
        }
        self.writer.submit(mutation)
    }

    /// Opens a read transaction pinned to the latest committed revision.
    ///
    /// # Errors
    ///
    /// Returns `persistence_failed` when SQLite cannot start the read.
    pub fn snapshot(&self) -> Result<Snapshot, StoreError> {
        Snapshot::begin(&self.db)
    }

    /// The limits this store advertises.
    pub const fn limits(&self) -> &StoreLimits {
        &self.limits
    }

    /// Finishes every queued mutation, then stops the writer.
    ///
    /// # Errors
    ///
    /// Returns `writer_panicked` when the writer thread panicked.
    pub fn close(self) -> Result<(), StoreError> {
        self.writer.shut_down()
    }

    fn start(
        db: PathBuf,
        mut connection: Connection,
        (version, options): (i64, StoreOptions),
    ) -> Result<Self, StoreError> {
        schema::prepare(&mut connection, version)?;
        let limits = options.limits.clone();
        let writer = Writer::spawn(connection, options)?;
        Ok(Self { db, limits, writer })
    }
}

/// Probes `db` and quarantines it when it cannot be used; `None` means there is no database.
/// The writable connection is opened only after the read-only probe classified the file.
fn settle(root: &Path, db: &Path) -> Result<Option<(Connection, i64)>, StoreError> {
    fs::create_dir_all(root).map_err(StoreError::io("create", root))?;
    match schema::probe(db)? {
        Probe::Missing => Ok(None),
        Probe::Ready { version } => Ok(Some((Connection::open(db)?, version))),
        Probe::Damaged(cause) => {
            let evidence =
                quarantine::preserve(root, cause, MaterialDisposition::MovedToQuarantine)?;
            Err(StoreError::Corrupted {
                evidence: Box::new(evidence),
            })
        }
        Probe::Newer(found) => {
            let cause = RecoveryCause::UnsupportedSchema {
                found,
                supported: SCHEMA_VERSION,
            };
            let evidence = match quarantine::find_incident(root, &cause)? {
                Some(earlier) => earlier,
                None => quarantine::preserve(root, cause, MaterialDisposition::CopiedOriginalKept)?,
            };
            Err(StoreError::Unsupported {
                evidence: Box::new(evidence),
            })
        }
    }
}

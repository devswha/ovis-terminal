//! Versioned schema: probing an existing file before touching it, then migrating.

use std::fs::File;
use std::io::{ErrorKind, Read as _};
use std::path::Path;

use rusqlite::{Connection, ErrorCode};

use crate::error::StoreError;
use crate::evidence::RecoveryCause;

/// The newest schema version this build reads and writes.
pub const SCHEMA_VERSION: i64 = 1;

const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// Migration `n` upgrades schema version `n` to `n + 1`.
const MIGRATIONS: [&str; 1] = ["
    CREATE TABLE head (
        id INTEGER PRIMARY KEY CHECK (id = 0),
        revision INTEGER NOT NULL
    );
    INSERT INTO head (id, revision) VALUES (0, 0);
    CREATE TABLE resources (
        kind TEXT NOT NULL,
        id TEXT NOT NULL,
        version INTEGER NOT NULL,
        value BLOB,
        revision INTEGER NOT NULL,
        PRIMARY KEY (kind, id)
    ) WITHOUT ROWID;
    CREATE TABLE events (
        seq INTEGER PRIMARY KEY,
        kind TEXT NOT NULL,
        data BLOB NOT NULL,
        op TEXT NOT NULL
    );
    CREATE TABLE receipts (
        op TEXT PRIMARY KEY,
        revision INTEGER NOT NULL,
        result BLOB NOT NULL
    ) WITHOUT ROWID;
    CREATE INDEX receipts_by_revision ON receipts (revision);
    CREATE TABLE floors (
        stream TEXT PRIMARY KEY,
        floor INTEGER NOT NULL
    ) WITHOUT ROWID;
    INSERT INTO floors (stream, floor) VALUES ('events', 0), ('receipts', 0);
    CREATE TABLE recovery_evidence (
        id TEXT PRIMARY KEY,
        evidence TEXT NOT NULL
    ) WITHOUT ROWID;
"];

/// What an existing database file turned out to be, decided before any write.
#[derive(Debug)]
pub(crate) enum Probe {
    Missing,
    Ready {
        connection: Connection,
        version: i64,
    },
    Damaged(RecoveryCause),
    Newer(i64),
}

pub(crate) fn probe(db: &Path) -> Result<Probe, StoreError> {
    let mut header = Vec::with_capacity(SQLITE_MAGIC.len());
    match File::open(db) {
        Ok(file) => file
            .take(16)
            .read_to_end(&mut header)
            .map_err(StoreError::io("read the header of", db))?,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Probe::Missing),
        Err(error) => return Err(StoreError::io("open", db)(error)),
    };
    // SQLite treats a zero-length file as an empty database; anything else must carry the magic.
    if !header.is_empty() && header.as_slice() != SQLITE_MAGIC.as_slice() {
        return Ok(Probe::Damaged(RecoveryCause::CorruptHeader));
    }
    match inspect(db) {
        Ok(probe) => Ok(probe),
        Err(error) => damage_of(&error).map_or(Err(StoreError::Sqlite(error)), |cause| {
            Ok(Probe::Damaged(cause))
        }),
    }
}

fn inspect(db: &Path) -> rusqlite::Result<Probe> {
    let connection = Connection::open(db)?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Ok(Probe::Newer(version));
    }
    if version < 0 {
        let detail = format!("negative schema version {version}");
        return Ok(Probe::Damaged(RecoveryCause::CorruptDatabase { detail }));
    }
    let verdict: String = connection.pragma_query_value(None, "quick_check", |row| row.get(0))?;
    if verdict != "ok" {
        return Ok(Probe::Damaged(RecoveryCause::CorruptDatabase {
            detail: verdict,
        }));
    }
    Ok(Probe::Ready {
        connection,
        version,
    })
}

fn damage_of(error: &rusqlite::Error) -> Option<RecoveryCause> {
    match error.sqlite_error_code() {
        Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt) => {
            Some(RecoveryCause::CorruptDatabase {
                detail: error.to_string(),
            })
        }
        _ => None,
    }
}

/// Switches to WAL with full fsync on commit, then applies every migration after `from`.
pub(crate) fn prepare(connection: &mut Connection, from: i64) -> Result<(), StoreError> {
    let mode: String =
        connection.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::WalUnavailable { mode });
    }
    connection.pragma_update(None, "synchronous", "FULL")?;
    let applied = usize::try_from(from).map_err(|_| StoreError::CounterOverflow {
        counter: "schema version",
    })?;
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(applied) {
        let next = index
            .checked_add(1)
            .and_then(|next| i64::try_from(next).ok())
            .ok_or(StoreError::CounterOverflow {
                counter: "schema version",
            })?;
        let transaction = connection.transaction()?;
        transaction.execute_batch(migration)?;
        transaction.pragma_update(None, "user_version", next)?;
        transaction.commit()?;
    }
    Ok(())
}

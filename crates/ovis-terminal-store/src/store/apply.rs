//! Writing one mutation inside an open transaction, then trimming to the retention limits.

use rusqlite::{OptionalExtension as _, Transaction, params};

use crate::error::StoreError;
use crate::ids::{Payload, Revision, Version};
use crate::mutation::{CasWrite, CommitReceipt, Mutation};
use crate::options::StoreLimits;

pub(super) fn apply(
    tx: &Transaction<'_>,
    mutation: &Mutation,
    limits: &StoreLimits,
) -> Result<CommitReceipt, StoreError> {
    let reused: Option<Revision> = tx
        .query_row(
            "SELECT revision FROM receipts WHERE op = ?1",
            [mutation.op.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    if reused.is_some() {
        return Err(StoreError::DuplicateOperation {
            op: mutation.op.clone(),
        });
    }
    let head: Revision = tx.query_row("SELECT revision FROM head", [], |row| row.get(0))?;
    let first_event = head.checked_add(1).ok_or(StoreError::CounterOverflow {
        counter: "revision",
    })?;
    let mut revision = head;
    for event in &mutation.events {
        revision = revision.checked_add(1).ok_or(StoreError::CounterOverflow {
            counter: "revision",
        })?;
        tx.execute(
            "INSERT INTO events (seq, kind, data, op) VALUES (?1, ?2, ?3, ?4)",
            params![
                revision,
                event.kind.as_str(),
                event.data.as_bytes(),
                mutation.op.as_str()
            ],
        )?;
    }
    let mut versions = Vec::with_capacity(mutation.writes.len());
    for write in &mutation.writes {
        versions.push((write.key.clone(), swap(tx, write, revision)?));
    }
    tx.execute(
        "INSERT INTO receipts (op, revision, result) VALUES (?1, ?2, ?3)",
        params![mutation.op.as_str(), revision, mutation.result.as_bytes()],
    )?;
    tx.execute("UPDATE head SET revision = ?1", [revision])?;
    trim(tx, Stream::Events, limits.max_events)?;
    trim(tx, Stream::Receipts, limits.max_receipts)?;
    Ok(CommitReceipt {
        revision,
        first_event,
        versions,
    })
}

fn swap(tx: &Transaction<'_>, write: &CasWrite, revision: Revision) -> Result<Version, StoreError> {
    let stored: Option<Version> = tx
        .query_row(
            "SELECT version FROM resources WHERE kind = ?1 AND id = ?2",
            [write.key.kind(), write.key.id()],
            |row| row.get(0),
        )
        .optional()?;
    let current = stored.unwrap_or(Version::ABSENT);
    if current != write.expected {
        return Err(StoreError::Conflict {
            key: write.key.clone(),
            expected: write.expected,
            current,
        });
    }
    let next = current.checked_next().ok_or(StoreError::CounterOverflow {
        counter: "resource version",
    })?;
    tx.execute(
        "INSERT INTO resources (kind, id, version, value, revision) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (kind, id) DO UPDATE
         SET version = excluded.version, value = excluded.value, revision = excluded.revision",
        params![
            write.key.kind(),
            write.key.id(),
            next,
            write.value.as_ref().map(Payload::as_bytes),
            revision
        ],
    )?;
    Ok(next)
}

#[derive(Debug, Clone, Copy)]
enum Stream {
    Events,
    Receipts,
}

impl Stream {
    const fn statements(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Events => (
                "SELECT COUNT(*) FROM events",
                "DELETE FROM events WHERE seq IN (SELECT seq FROM events ORDER BY seq LIMIT ?1)",
                "SELECT MIN(seq) FROM events",
            ),
            Self::Receipts => (
                "SELECT COUNT(*) FROM receipts",
                "DELETE FROM receipts WHERE op IN (SELECT op FROM receipts ORDER BY revision LIMIT ?1)",
                "SELECT MIN(revision) FROM receipts",
            ),
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Events => "events",
            Self::Receipts => "receipts",
        }
    }
}

/// Evicts the oldest rows beyond `retained` and records the lowest retained revision as the floor.
fn trim(tx: &Transaction<'_>, stream: Stream, retained: u64) -> Result<(), StoreError> {
    let (count_sql, evict_sql, lowest_sql) = stream.statements();
    let count: i64 = tx.query_row(count_sql, [], |row| row.get(0))?;
    let limit = i64::try_from(retained).unwrap_or(i64::MAX);
    let Some(excess) = count.checked_sub(limit).filter(|excess| *excess > 0) else {
        return Ok(());
    };
    tx.execute(evict_sql, [excess])?;
    let lowest: Option<Revision> = tx.query_row(lowest_sql, [], |row| row.get(0))?;
    let floor = if let Some(lowest) = lowest {
        lowest
    } else {
        let head: Revision = tx.query_row("SELECT revision FROM head", [], |row| row.get(0))?;
        head.checked_add(1).ok_or(StoreError::CounterOverflow {
            counter: "revision",
        })?
    };
    tx.execute(
        "UPDATE floors SET floor = ?1 WHERE stream = ?2",
        params![floor, stream.name()],
    )?;
    Ok(())
}

//! The per-session start lock: one claimant at a time may judge, clear and bind the
//! endpoint, so two concurrent starts can never both replace a stale endpoint.
//!
//! The lock file lives in the owner-only service directory and is never removed:
//! removing a lock file lets a later claimant lock a different inode than a waiter.

use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt as _;

use rustix::fs::{FlockOperation, flock};

use super::{ServiceError, ServicePaths, record};

/// An exclusive `flock` on the session's lock file, released on drop.
#[derive(Debug)]
pub(super) struct StartLock {
    _held: File,
}

impl StartLock {
    /// Creates the service directory and lock file if needed and waits for the lock.
    pub(super) fn acquire(paths: &ServicePaths) -> Result<Self, ServiceError> {
        record::create_directory(paths.directory())?;
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(paths.lock())
            .map_err(|source| ServiceError::Io {
                operation: "open the service start lock",
                source,
            })?;
        flock(&file, FlockOperation::LockExclusive).map_err(|source| ServiceError::Io {
            operation: "take the service start lock",
            source: source.into(),
        })?;
        Ok(Self { _held: file })
    }
}

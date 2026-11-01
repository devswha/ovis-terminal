//! The shared handle every source commits through.

use std::sync::Arc;

use ovis_terminal_store::Snapshot;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Semaphore;

use super::{CommitError, CommitSource, Committed, remote};
use crate::events::{Commit, Coordinator, EventData, SnapshotProducer};

/// One coordinator and its bounded blocking writer submissions, shared by every source.
#[derive(Debug)]
pub struct Boundary<D, P> {
    core: Arc<Coordinator<D, P>>,
    slots: Arc<Semaphore>,
    capacity: usize,
}

impl<D, P> Clone for Boundary<D, P> {
    fn clone(&self) -> Self {
        Self {
            core: Arc::clone(&self.core),
            slots: Arc::clone(&self.slots),
            capacity: self.capacity,
        }
    }
}

impl<D, P> Boundary<D, P> {
    /// Shares `core`, admitting at most `capacity` asynchronous submissions at once.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` for a zero capacity.
    pub fn new(core: Arc<Coordinator<D, P>>, capacity: usize) -> Result<Self, CommitError> {
        if capacity == 0 {
            return Err(CommitError::InvalidBound {
                bound: "writer submissions",
            });
        }
        Ok(Self {
            core,
            slots: Arc::new(Semaphore::new(capacity)),
            capacity,
        })
    }

    /// The coordinator snapshots and subscriptions are installed against.
    pub const fn core(&self) -> &Arc<Coordinator<D, P>> {
        &self.core
    }
}

impl<D, P> Boundary<D, P>
where
    D: EventData + Clone + Serialize + DeserializeOwned,
    P: SnapshotProducer,
{
    /// Commits `build`'s unit from `source` on the calling blocking worker.
    ///
    /// The sequence is allocated inside the serialized durable commit and the record is
    /// published only after the store acknowledges it. A remote source's upstream
    /// position is attached and advanced in the same transaction.
    ///
    /// # Errors
    ///
    /// Returns the builder's error without submitting, `remote_replayed` for an upstream
    /// position that is not new, `storage_full`, or another typed store failure.
    pub fn commit<F>(&self, source: CommitSource, build: F) -> Result<Committed, CommitError>
    where
        F: FnOnce(&Snapshot) -> Result<Commit<D>, CommitError>,
    {
        let cursor = self
            .core
            .commit_unit(|snapshot| {
                let commit = build(snapshot)?;
                match &source {
                    CommitSource::Remote { origin } => Ok(commit
                        .write(remote::advance(snapshot, origin)?)
                        .mirrored_from(origin.clone())),
                    CommitSource::Terminal { .. } | CommitSource::Registry => Ok(commit),
                }
            })
            .map_err(|error| match error {
                CommitError::Event(event) => CommitError::from_event(event),
                other => other,
            })?;
        Ok(Committed { cursor, source })
    }
}

impl<D, P> Boundary<D, P>
where
    D: EventData + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
{
    /// Commits from an async task: the blocking commit runs on a bounded blocking worker,
    /// so no lock is held across an await.
    ///
    /// # Errors
    ///
    /// Returns `backpressure` when every submission slot is taken, the commit's own
    /// error, or `unavailable` when the worker stopped.
    pub async fn submit<F>(&self, source: CommitSource, build: F) -> Result<Committed, CommitError>
    where
        F: FnOnce(&Snapshot) -> Result<Commit<D>, CommitError> + Send + 'static,
    {
        let slot =
            Arc::clone(&self.slots)
                .try_acquire_owned()
                .map_err(|_| CommitError::Backpressure {
                    slots: self.capacity,
                })?;
        let boundary = self.clone();
        tokio::task::spawn_blocking(move || {
            let committed = boundary.commit(source, build);
            drop(slot);
            committed
        })
        .await?
    }
}

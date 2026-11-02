//! The lease journal of the boot serving now. A planned restart closes the old boot's
//! journal and opens one under the successor identity, so every lease event names the boot
//! that committed it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ovis_terminal_core::events::{Coordinator, EventError};
use ovis_terminal_core::leases::Transition;
use ovis_terminal_core::registry::RuntimeIdentity;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_server::streams::{LeaseJournal, StreamError};
use ovis_terminal_store::OpId;

use super::AssemblyError;
use super::boot::{LeaseLog, NoSections, store};
use super::lanes::DispatchLog;

/// The lease log of the current boot, absent once the lifetime stopped.
#[derive(Debug)]
pub(super) struct BootJournal {
    state: PathBuf,
    log: Mutex<Option<LeaseLog>>,
    /// The serving boot's dispatch log, where committed lease events reach subscribers.
    feed: Mutex<Option<Arc<DispatchLog>>>,
}

impl BootJournal {
    /// Journals into `log`, which lives below `state`.
    pub(super) fn new(state: &Path, log: LeaseLog) -> Self {
        Self {
            state: state.to_path_buf(),
            log: Mutex::new(Some(log)),
            feed: Mutex::new(None),
        }
    }

    /// Publishes later lease events to `feed`, or to no subscriber once the boot ends.
    pub(super) fn publish_to(&self, feed: Option<Arc<DispatchLog>>) {
        *self.feed.lock().unwrap_or_else(PoisonError::into_inner) = feed;
    }

    /// Each update replaces the whole slot, so a poisoned lock still holds a usable value.
    fn slot(&self) -> MutexGuard<'_, Option<LeaseLog>> {
        self.log.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Closes the old boot's log and opens one stamped with `identity`.
    pub(super) fn reopen(&self, identity: RuntimeIdentity) -> Result<(), AssemblyError> {
        let old = self.slot().take();
        if let Some(old) = old {
            old.close()
                .map_err(AssemblyError::stop("close the lease log of the old boot"))?;
        }
        let store = store(&self.state, "leases.db")?;
        *self.slot() = Some(Coordinator::new(store, identity, NoSections));
        Ok(())
    }

    /// The lease log's current position: the sequence a read of the lease state reflects.
    pub(super) fn cursor(&self) -> Result<EventSeq, StreamError> {
        let slot = self.slot();
        let log = slot
            .as_ref()
            .ok_or(StreamError::Journal(EventError::AdmissionWithdrawn))?;
        let pinned = log.pin_snapshot().map_err(StreamError::Journal)?;
        let seq = EventSeq::new(pinned.cursor.seq.get());
        log.release_snapshot(&pinned.token)
            .map_err(StreamError::Journal)?;
        drop(slot);
        Ok(seq)
    }

    /// Closes the log; later transitions are refused as withdrawn.
    pub(super) fn close(&self) -> Result<(), AssemblyError> {
        let log = self.slot().take();
        log.map_or(Ok(()), |log| {
            log.close()
                .map_err(AssemblyError::stop("close the lease log"))
        })
    }
}

impl LeaseJournal for BootJournal {
    /// Commits to the lease log first; only a committed transition is then published.
    fn record(&self, op: OpId, transition: Transition) -> Result<Option<EventSeq>, StreamError> {
        let committed = self.slot().as_ref().map_or_else(
            || Err(StreamError::Journal(EventError::AdmissionWithdrawn)),
            |log| log.record(op.clone(), transition.clone()),
        )?;
        let feed = self
            .feed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(feed) = feed {
            feed.record(op, transition)?;
        }
        Ok(committed)
    }
}

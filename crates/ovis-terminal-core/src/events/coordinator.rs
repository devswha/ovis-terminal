//! The single synchronous commit boundary used by blocking core workers.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use ovis_terminal_protocol::schema::scalar::{ByteCount, ByteOffset, EventSeq};
use ovis_terminal_store::{EventKind, Mutation, NewEvent, Payload, Store};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::delivery::{Charge, Sink};
use super::record::{Body, SCHEMA_VERSION};
use super::snapshot::{PageCount, Pinned, SnapshotDigest};
use super::{
    Commit, Cursor, EventData, EventError, Filter, SnapshotLease, SnapshotPage, SnapshotProducer,
    SnapshotToken, StateSnapshot,
};
use crate::registry::RuntimeIdentity;

/// Admission bounds; aggregate queue and pinned-snapshot budgets are separate.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Limits {
    /// Maximum independent subscriptions.
    pub subscriptions: usize,
    /// Maximum queued events per subscription.
    pub queued_events: usize,
    /// Aggregate bytes charged to all queued event copies.
    pub queued_bytes: usize,
    /// Maximum simultaneously pinned snapshots.
    pub pinned_snapshots: usize,
    /// Aggregate serialized snapshot byte budget.
    pub snapshot_bytes: usize,
    /// Maximum bytes returned by one snapshot page.
    pub page_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            subscriptions: 64,
            queued_events: 10_000,
            queued_bytes: 64 << 20,
            pinned_snapshots: 8,
            snapshot_bytes: 64 << 20,
            page_bytes: 64 << 10,
        }
    }
}

#[derive(Debug)]
pub(super) struct State<D> {
    pub store: Store,
    pub sinks: Vec<Sink<D>>,
    pub pages: BTreeMap<SnapshotToken, Pinned>,
}

/// Serializes mutations, snapshot capture and subscription installation.
///
/// Call from dedicated blocking workers. A submitted commit runs to completion
/// even when its caller disconnects; publication follows the durable writer's
/// commit receipt. No live actor state is read across this boundary.
#[derive(Debug)]
pub struct Coordinator<D, P> {
    pub(super) state: Mutex<State<D>>,
    pub(super) producer: P,
    pub(super) identity: RuntimeIdentity,
    pub(super) limits: Limits,
    pub(super) queued_bytes: Arc<AtomicUsize>,
}

impl<D, P> Coordinator<D, P> {
    /// Takes ownership of the core's store after identity/bootstrap reservation.
    pub fn new(store: Store, identity: RuntimeIdentity, producer: P) -> Self {
        Self {
            state: Mutex::new(State {
                store,
                sinks: Vec::new(),
                pages: BTreeMap::new(),
            }),
            producer,
            identity,
            limits: Limits::default(),
            queued_bytes: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Configures bounds before sharing the coordinator.
    ///
    /// # Errors
    ///
    /// Returns `limit_exceeded` for zero bounds or pages exceeding 1 MiB.
    pub fn with_limits(mut self, limits: Limits) -> Result<Self, EventError> {
        if [
            limits.subscriptions,
            limits.queued_events,
            limits.queued_bytes,
            limits.pinned_snapshots,
            limits.snapshot_bytes,
            limits.page_bytes,
        ]
        .contains(&0)
            || limits.page_bytes > 1 << 20
        {
            return Err(EventError::LimitExceeded);
        }
        self.limits = limits;
        Ok(self)
    }

    pub(super) fn lock(&self) -> Result<MutexGuard<'_, State<D>>, EventError> {
        self.state.lock().map_err(|_| EventError::Poisoned)
    }

    /// Releases page leases and delivery channels, then joins the durable writer.
    ///
    /// # Errors
    ///
    /// Returns `coordinator_stopped` after poisoning, or the store's shutdown error.
    pub fn close(self) -> Result<(), EventError> {
        let State {
            store,
            sinks,
            pages,
        } = self.state.into_inner().map_err(|_| EventError::Poisoned)?;
        drop(sinks);
        drop(pages);
        Ok(store.close()?)
    }

    pub(super) const fn cursor(&self, seq: u64) -> Cursor {
        Cursor {
            machine_id: self.identity.machine_id(),
            runtime_id: self.identity.runtime_id(),
            seq: EventSeq::new(seq),
        }
    }

    pub(super) fn charge(&self, bytes: usize) -> Option<Charge> {
        let mut current = self.queued_bytes.load(Ordering::SeqCst);
        loop {
            let next = current
                .checked_add(bytes)
                .filter(|total| *total <= self.limits.queued_bytes)?;
            match self.queued_bytes.compare_exchange_weak(
                current,
                next,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {
                    return Some(Charge {
                        bytes,
                        budget: Arc::clone(&self.queued_bytes),
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }
}

impl<D: EventData + Clone + Serialize + DeserializeOwned, P: SnapshotProducer> Coordinator<D, P> {
    /// Commits state, receipt and typed event; publishes only after SQLite COMMIT.
    ///
    /// The event's sequence is the store row's sequence, not a preallocated or
    /// in-memory counter. Its remaining envelope fields are the same row's payload.
    ///
    /// # Errors
    ///
    /// Returns typed serialization, admission, CAS or persistence failures.
    pub fn commit(&self, commit: Commit<D>) -> Result<Cursor, EventError> {
        if commit.affected.is_empty() {
            return Err(EventError::MissingResources);
        }
        let mut state = self.lock()?;
        let body = Body {
            schema_version: SCHEMA_VERSION,
            identity: self.identity,
            affected: commit.affected,
            origin: commit.origin,
            data: commit.data,
        };
        let bytes = serde_json::to_vec(&body)?;
        // Includes the encoded body and an upper bound for sequence/type/envelope fields.
        let charged = bytes.len().saturating_add(1024);
        let event = NewEvent::new(EventKind::new(body.data.event_type()), Payload::from(bytes));
        let mut mutation = Mutation::new(commit.op, Payload::from("{}"), event);
        for write in commit.writes {
            mutation = mutation.compare_and_swap(write);
        }
        let receipt = state.store.submit(mutation)?.wait()?;
        let event = body.into_event(EventSeq::new(receipt.first_event.get()));
        state.sinks.retain(|sink| sink.lost.strong_count() > 0);
        for sink in &state.sinks {
            if sink.filter.matches(&event) {
                match self.charge(charged) {
                    Some(charge) => sink.publish(&event, charge),
                    None => {
                        if let Some(lost) = sink.lost.upgrade() {
                            lost.store(true, Ordering::SeqCst);
                        }
                    }
                }
            }
        }
        drop(state);
        Ok(event.cursor)
    }

    /// Captures implemented sections and pins their complete serialized representation.
    ///
    /// # Errors
    ///
    /// Returns typed producer/store failures or `limit_exceeded` at the pin budget.
    pub fn pin_snapshot(&self) -> Result<SnapshotLease, EventError> {
        let mut state = self.lock()?;
        let view = state.store.snapshot()?;
        let cursor = self.cursor(view.revision().get());
        let snapshot = StateSnapshot {
            cursor,
            sections: self.producer.capture(&view, &Filter::default())?,
        };
        let pinned = self.pin(&mut state, &snapshot);
        drop(state);
        pinned
    }

    pub(super) fn pin(
        &self,
        state: &mut State<D>,
        snapshot: &StateSnapshot<P::Sections>,
    ) -> Result<SnapshotLease, EventError> {
        if state.pages.len() >= self.limits.pinned_snapshots {
            return Err(EventError::LimitExceeded);
        }
        let view = state.store.snapshot()?;
        let cursor = snapshot.cursor;
        let bytes = serde_json::to_vec(snapshot)?;
        let used = state
            .pages
            .values()
            .map(|page| page.bytes.len())
            .sum::<usize>();
        if bytes.len() > self.limits.snapshot_bytes.saturating_sub(used) {
            return Err(EventError::LimitExceeded);
        }
        let total =
            ByteCount::new(u64::try_from(bytes.len()).map_err(|_| EventError::LimitExceeded)?);
        let page_count = PageCount::for_bytes(bytes.len(), self.limits.page_bytes)?;
        let digest = SnapshotDigest::for_bytes(&bytes);
        let token = SnapshotToken::new();
        state.pages.insert(
            token.clone(),
            Pinned {
                _view: view,
                token: token.clone(),
                cursor,
                bytes,
                page_count,
                digest: digest.clone(),
            },
        );
        Ok(SnapshotLease {
            token,
            cursor,
            total,
            page_count,
            digest,
        })
    }

    /// Reads one bounded page of a pinned revision.
    ///
    /// # Errors
    ///
    /// Returns `stale_snapshot` after release/restart, or `invalid_params` for an invalid offset.
    pub fn page(
        &self,
        token: &SnapshotToken,
        offset: ByteOffset,
    ) -> Result<SnapshotPage, EventError> {
        let state = self.lock()?;
        let pinned = state.pages.get(token).ok_or(EventError::StaleSnapshot)?;
        let page = pinned.page(offset, self.limits.page_bytes);
        drop(state);
        page
    }

    /// Releases the read transaction and its immutable bytes.
    ///
    /// # Errors
    ///
    /// Returns `stale_snapshot` if the lease is not owned by this core.
    pub fn release_snapshot(&self, token: &SnapshotToken) -> Result<(), EventError> {
        self.lock()?
            .pages
            .remove(token)
            .ok_or(EventError::StaleSnapshot)?;
        Ok(())
    }
}

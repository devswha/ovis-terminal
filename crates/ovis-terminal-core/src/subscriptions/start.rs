//! Installing one subscription atomically with its snapshot, head or cursor replay.

use std::sync::Arc;
use std::sync::mpsc::TryRecvError;

use ovis_terminal_protocol::schema::ids::SubscriptionId;
use ovis_terminal_protocol::schema::methods::events::{EventsSubscribeParams, MissedReason};
use ovis_terminal_protocol::schema::scalar::EventSeq;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::fanout::Offered;
use super::table::{Breach, Entry, Table};
use super::{Feed, LossCause, LossRecord, QueueLimits, Selector, SubscriptionError, Subscriptions};
use crate::events::{Cursor, EventData, EventError, SnapshotProducer, StateSnapshot};
use crate::registry::MachineId;

/// Where a new subscription's delivery begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Start {
    /// A complete snapshot at revision R, then matching commits after R.
    Snapshot,
    /// No snapshot; matching commits after the current fan-out head.
    Head,
    /// Matching retained commits after this cursor, or a fresh snapshot when it cannot resume.
    Resume(Cursor),
}

/// Parameters of one independent subscription.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Request {
    /// Delivery start.
    pub start: Start,
    /// Event selection, replaceable later without interrupting delivery.
    pub selector: Selector,
}

impl Request {
    /// Combines a start and a selection.
    pub const fn new(start: Start, selector: Selector) -> Self {
        Self { start, selector }
    }

    /// Decodes `events.subscribe` parameters for the runtime on `machine`.
    ///
    /// A cursor resumes even when `snapshot` is set; a snapshot then arrives only when the
    /// cursor cannot resume.
    ///
    /// # Errors
    ///
    /// Returns `filter_invalid` for an unusable filter or `invalid_params` for a cursor
    /// whose runtime lifetime is malformed.
    pub fn decode(
        params: &EventsSubscribeParams,
        machine: MachineId,
    ) -> Result<Self, SubscriptionError> {
        let selector = Selector::decode(&params.filter)?;
        let start = match &params.cursor {
            Some(cursor) => Start::Resume(Cursor::new(
                machine,
                cursor
                    .runtime_lifetime
                    .as_str()
                    .parse()
                    .map_err(SubscriptionError::InvalidCursor)?,
                cursor.seq,
            )),
            None if params.snapshot => Start::Snapshot,
            None => Start::Head,
        };
        Ok(Self::new(start, selector))
    }
}

/// An installed subscription and what its consumer must apply before its first batch.
#[derive(Debug)]
#[non_exhaustive]
pub struct Started<D, P: SnapshotProducer> {
    /// Pull handle; dropping it closes only this subscription.
    pub feed: Feed<D, P>,
    /// Complete state when requested or when the cursor could not resume.
    pub snapshot: Option<StateSnapshot<P::Sections>>,
    /// Sequence the consumer's state reflects; every later batch is strictly after it.
    pub snapshot_seq: EventSeq,
    /// Lowest cursor sequence the durable log can still replay from.
    pub replay_floor: EventSeq,
    /// Why the cursor could not resume, when it could not.
    pub missed: Option<MissedReason>,
    /// The bounds this subscription runs under.
    pub limits: QueueLimits,
}

struct Opened<S> {
    snapshot: Option<StateSnapshot<S>>,
    seq: u64,
    missed: Option<MissedReason>,
}

impl<D, P> Subscriptions<D, P>
where
    D: EventData + Clone + Serialize + DeserializeOwned + From<LossRecord>,
    P: SnapshotProducer,
{
    /// Installs an independent subscription; no other subscription is touched.
    ///
    /// # Errors
    ///
    /// Returns `subscription_limit`, `seq_ahead` for a cursor past the committed head,
    /// or a typed coordinator failure; nothing is installed on error.
    pub fn subscribe(&self, request: Request) -> Result<Started<D, P>, SubscriptionError> {
        let replay_floor = self.shared.core.replay_floor()?;
        let id = SubscriptionId::new(uuid::Uuid::now_v7().to_string());
        let mut table = self.shared.lock()?;
        if self.stopped() {
            return Err(SubscriptionError::Stopped);
        }
        table.reserve()?;
        let opened = match request.start {
            Start::Snapshot => {
                let snapshot = self.shared.capture()?;
                let seq = snapshot.cursor.seq.get();
                table.insert(Entry::new(id.clone(), request.selector, seq))?;
                Opened {
                    snapshot: Some(snapshot),
                    seq,
                    missed: None,
                }
            }
            Start::Head => {
                let seq = table.head;
                table.insert(Entry::new(id.clone(), request.selector, seq))?;
                Opened {
                    snapshot: None,
                    seq,
                    missed: None,
                }
            }
            Start::Resume(cursor) => {
                let entry = Entry::new(id.clone(), request.selector, cursor.seq.get());
                self.resume(&mut table, Resuming { entry, cursor })?
            }
        };
        drop(table);
        Ok(Started {
            feed: Feed {
                shared: Arc::clone(&self.shared),
                id,
            },
            snapshot: opened.snapshot,
            snapshot_seq: EventSeq::new(opened.seq),
            replay_floor,
            missed: opened.missed,
            limits: self.limits(),
        })
    }

    /// Replays retained commits after the cursor into the new entry under the table lock;
    /// later commits arrive through fan-out above the entry's scan cursor.
    fn resume(
        &self,
        table: &mut Table<D>,
        resuming: Resuming<D>,
    ) -> Result<Opened<P::Sections>, SubscriptionError> {
        let Resuming { entry, cursor } = resuming;
        let replay = self.shared.core.subscribe(Some(cursor))?;
        if let Some(missed) = replay.missed {
            let snapshot = replay.snapshot.ok_or(SubscriptionError::Stopped)?;
            let seq = snapshot.cursor.seq.get();
            table.insert(Entry::new(entry.id, entry.selector, seq))?;
            return Ok(Opened {
                snapshot: Some(snapshot),
                seq,
                missed: Some(missed),
            });
        }
        let id = entry.id.clone();
        let index = table.insert(entry)?;
        let replayed = loop {
            match replay.delivery.try_recv() {
                Ok(event) => match serde_json::to_vec(&event) {
                    Ok(bytes) => table.offer(
                        index,
                        Offered {
                            event: &event,
                            bytes: bytes.len(),
                        },
                    ),
                    Err(error) => break Err(EventError::from(error)),
                },
                Err(EventError::TryReceive(TryRecvError::Empty)) => break Ok(()),
                Err(EventError::Missed) => {
                    let at = cursor.seq.get().saturating_add(1);
                    table.lose(
                        index,
                        Breach {
                            cause: LossCause::QueueOverflow,
                            at,
                        },
                    );
                    break Ok(());
                }
                Err(other) => break Err(other),
            }
        };
        if let Err(error) = replayed {
            table.remove(&id);
            return Err(error.into());
        }
        Ok(Opened {
            snapshot: None,
            seq: cursor.seq.get(),
            missed: None,
        })
    }
}

struct Resuming<D> {
    entry: Entry<D>,
    cursor: Cursor,
}

//! Capture and subscribe under the same commit exclusion boundary.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::sync_channel;

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::coordinator::State;
use super::delivery::Sink;
use super::record::subscription_id;
use super::replay::Replay;
use super::{
    Coordinator, Cursor, Delivery, EventData, EventError, Filter, PagedSubscription,
    SnapshotProducer, StateSnapshot, Subscription,
};

/// Startup cursor and matching resources shared by inline and paged installation.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct SubscribeRequest {
    /// Last processed cursor; absent starts from a snapshot.
    pub cursor: Option<Cursor>,
    /// Independent event and snapshot selection.
    pub filter: Filter,
}

impl SubscribeRequest {
    /// Selects a resume cursor and the independent filter to install.
    pub const fn new(cursor: Option<Cursor>, filter: Filter) -> Self {
        Self { cursor, filter }
    }
}

impl<D: EventData + Clone + Serialize + DeserializeOwned, P: SnapshotProducer> Coordinator<D, P> {
    /// Installs an independent unfiltered subscription with a snapshot or retained replay.
    ///
    /// # Errors
    ///
    /// Returns the same typed admission/store/producer failures as `subscribe_matching`.
    pub fn subscribe(
        &self,
        cursor: Option<Cursor>,
    ) -> Result<Subscription<D, P::Sections>, EventError> {
        self.subscribe_matching(SubscribeRequest {
            cursor,
            filter: Filter::default(),
        })
    }

    /// Atomically captures state at R and installs delivery for matching commits after R.
    ///
    /// A valid retained cursor returns replay without a snapshot. A mismatched or
    /// expired cursor returns a fresh snapshot and a typed loss reason instead.
    /// Dropping delivery disconnects only this subscription.
    ///
    /// # Errors
    ///
    /// Returns typed producer/store failures, `seq_ahead`, or `limit_exceeded`.
    pub fn subscribe_matching(
        &self,
        request: SubscribeRequest,
    ) -> Result<Subscription<D, P::Sections>, EventError> {
        let mut state = self.lock()?;
        self.start(&mut state, request)
    }

    /// Installs delivery atomically with a stable, explicitly released page lease.
    ///
    /// # Errors
    ///
    /// Returns typed startup failures or `limit_exceeded` without leaving a subscription.
    pub fn subscribe_paged(
        &self,
        request: SubscribeRequest,
    ) -> Result<PagedSubscription<D>, EventError> {
        let mut state = self.lock()?;
        let subscription = self.start(&mut state, request)?;
        let snapshot = match subscription.snapshot {
            Some(snapshot) => Some(self.pin(&mut state, &snapshot)?),
            None => None,
        };
        drop(state);
        Ok(PagedSubscription {
            id: subscription.id,
            snapshot,
            missed: subscription.missed,
            delivery: subscription.delivery,
        })
    }

    fn start(
        &self,
        state: &mut State<D>,
        request: SubscribeRequest,
    ) -> Result<Subscription<D, P::Sections>, EventError> {
        let SubscribeRequest { cursor, filter } = request;
        state.sinks.retain(|sink| sink.lost.strong_count() > 0);
        if state.sinks.len() >= self.limits.subscriptions {
            return Err(EventError::LimitExceeded);
        }
        let view = state.store.snapshot()?;
        let replay = match cursor {
            Some(cursor) => self.replay(&view, cursor)?,
            None => Replay {
                events: Vec::new(),
                missed: None,
            },
        };
        let snapshot = if cursor.is_none() || replay.missed.is_some() {
            Some(StateSnapshot {
                cursor: self.cursor(view.revision().get()),
                sections: self.producer.capture(&view, &filter)?,
            })
        } else {
            None
        };
        let lost = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = sync_channel(self.limits.queued_events);
        let sink = Sink {
            sender,
            lost: Arc::downgrade(&lost),
            filter,
        };
        let delivery = Delivery { receiver, lost };
        for event in replay.events {
            if sink.filter.matches(&event) {
                let bytes = serde_json::to_vec(&event)?.len().saturating_add(1024);
                sink.publish(&event, self.charge(bytes).ok_or(EventError::LimitExceeded)?);
            }
        }
        let id = subscription_id();
        state.sinks.push(sink);
        Ok(Subscription {
            id,
            snapshot,
            missed: replay.missed,
            delivery,
        })
    }
}

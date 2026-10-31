//! The dedicated fan-out worker: drains the coordinator's unfiltered delivery.

use std::sync::Weak;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::shared::Shared;
use super::{LossRecord, SubscriptionError};
use crate::events::{Delivery, EventData, EventError, SnapshotProducer};

/// How long the worker waits for a commit before it rechecks for shutdown.
const TICK: Duration = Duration::from_millis(50);

/// Wakes consumers when the worker leaves for any reason, including a panic.
struct Exit<D, P>(Weak<Shared<D, P>>);

impl<D, P> Drop for Exit<D, P> {
    fn drop(&mut self) {
        if let Some(shared) = self.0.upgrade() {
            shared.stop();
        }
    }
}

/// Runs until shutdown, until every handle is gone, or until a durable write fails.
///
/// An idle tick also commits markers owed by consumers that went away without a commit
/// following them.
pub(super) fn run<D, P>(
    shared: &Weak<Shared<D, P>>,
    mut delivery: Delivery<D>,
) -> Result<(), SubscriptionError>
where
    D: EventData + Clone + Serialize + DeserializeOwned + From<LossRecord>,
    P: SnapshotProducer,
{
    let _exit = Exit(Weak::clone(shared));
    loop {
        let Some(hub) = shared.upgrade() else {
            return Ok(());
        };
        if hub.stopped.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        match delivery.recv_timeout(TICK) {
            Ok(event) => hub.deliver(&event)?,
            Err(EventError::Receive(RecvTimeoutError::Timeout)) => {
                if hub.owes()? {
                    hub.persist_pending()?;
                }
            }
            Err(EventError::Missed) => delivery = hub.refill()?,
            Err(EventError::Receive(RecvTimeoutError::Disconnected)) => {
                return Err(SubscriptionError::Stopped);
            }
            Err(other) => return Err(other.into()),
        }
    }
}

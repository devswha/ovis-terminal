//! Advertised per-subscription and session-wide delivery bounds.

use std::time::Duration;

use ovis_terminal_protocol::schema::limits::Limits;

use super::SubscriptionError;

/// The longest idle interval a subscription may go without a heartbeat.
pub const MAX_HEARTBEAT: Duration = Duration::from_secs(15);

/// Bounds each subscription's queue independently; the session budget only evicts the
/// largest queue, never a healthy one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueueLimits {
    /// Simultaneously open subscriptions.
    pub subscriptions: usize,
    /// Undelivered events one subscription may hold.
    pub queue_events: usize,
    /// Undelivered serialized bytes one subscription may hold.
    pub queue_bytes: usize,
    /// Undelivered serialized bytes all subscriptions may hold together.
    pub session_bytes: usize,
    /// Idle interval after which a subscription reports a heartbeat.
    pub heartbeat: Duration,
}

impl QueueLimits {
    /// The advertised v1 bounds: 64 subscriptions, 10,000 events or 64 MiB per queue,
    /// the 64 MiB session event-queue budget and a 15 s heartbeat.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` if an advertised bound does not fit this platform.
    pub fn v1() -> Result<Self, SubscriptionError> {
        let advertised = Limits::v1();
        let size =
            |value: u64| usize::try_from(value).map_err(|_| SubscriptionError::InvalidLimits);
        Self {
            subscriptions: 64,
            queue_events: size(advertised.event_replay_events.get())?,
            queue_bytes: size(advertised.event_replay_bytes.get())?,
            session_bytes: size(advertised.session_event_queue_bytes.get())?,
            heartbeat: Duration::from_millis(advertised.heartbeat_ms.get()),
        }
        .checked()
    }

    /// Validates bounds chosen by assembly or a test fixture.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` for a zero bound or a heartbeat above 15 seconds.
    pub fn checked(self) -> Result<Self, SubscriptionError> {
        let zero = [
            self.subscriptions,
            self.queue_events,
            self.queue_bytes,
            self.session_bytes,
        ]
        .contains(&0);
        if zero || self.heartbeat.is_zero() || self.heartbeat > MAX_HEARTBEAT {
            return Err(SubscriptionError::InvalidLimits);
        }
        Ok(self)
    }
}

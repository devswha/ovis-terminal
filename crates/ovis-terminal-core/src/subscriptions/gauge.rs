//! What one subscription reports to the health consumer table, when the hub reports.
//!
//! Queue bytes rise when fan-out copies an event in and fall when the consumer takes
//! its batch or the queue is dropped by a loss or a close. A loss names the matching
//! sequences known to be dropped; a restart from a fresh snapshot marks that range
//! resynchronized.

use ovis_terminal_protocol::domain::health::{ConsumerRef, LossRange};
use ovis_terminal_protocol::schema::ids::{ClientId, SubscriptionId};
use ovis_terminal_protocol::schema::scalar::EventSeq;

use super::table::{Entry, Lost, Phase, Table};
use crate::health::Health;

fn wide(bytes: usize) -> u64 {
    u64::try_from(bytes).unwrap_or(u64::MAX)
}

const fn range(lost: Lost) -> LossRange {
    LossRange::Events {
        first_seq: lost.from,
        last_seq: lost.through,
    }
}

impl<D> Entry<D> {
    pub(super) fn report_queued(&self, bytes: usize) {
        if let Some(gauge) = &self.gauge {
            gauge.queued_now(wide(bytes));
        }
    }

    pub(super) fn report_taken(&self, bytes: usize) {
        if let Some(gauge) = &self.gauge {
            gauge.written(wide(bytes));
        }
    }

    pub(super) fn report_dropped(&self, bytes: usize) {
        if let Some(gauge) = &self.gauge {
            gauge.skipped(wide(bytes));
        }
    }

    pub(super) fn report_lost(&self, lost: Lost) {
        if let Some(gauge) = &self.gauge {
            gauge.lost(range(lost));
        }
    }

    pub(super) fn report_resynced(&self, lost: Lost) {
        if let Some(gauge) = &self.gauge {
            gauge.resynced(range(lost));
        }
    }
}

impl<D> Table<D> {
    /// Starts reporting subscription `id` of `client` to `health`, carrying over what a
    /// replay already queued or lost. A subscription that names no client, or that the
    /// bounded table refuses, runs untracked and is counted as such.
    pub(super) fn track(
        &mut self,
        id: &SubscriptionId,
        (health, client): (Option<&Health>, Option<&ClientId>),
    ) {
        let Some(health) = health else {
            return;
        };
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == *id) else {
            return;
        };
        let Some(client) = client else {
            health.consumers().untracked();
            return;
        };
        let consumer = ConsumerRef::Subscription {
            subscription_id: id.clone(),
        };
        entry.gauge = health.consumers().track(client.clone(), consumer).ok();
        entry.report_queued(entry.bytes);
        if let Phase::Lost(lost) = entry.phase {
            entry.report_lost(lost);
        }
    }
}

/// The last dropped sequence of a loss starting at `from` while `at` was offered.
pub(super) const fn through(from: EventSeq, at: u64) -> EventSeq {
    if at > from.get() {
        EventSeq::new(at)
    } else {
        from
    }
}

//! Durable loss markers committed before a lossy subscription is recovered or closed.

use ovis_terminal_protocol::schema::ids::SubscriptionId;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_store::OpId;
use serde::{Deserialize, Serialize};

use crate::events::{Commit, ResourceId};
use crate::registry::MachineId;

/// Why a subscription stopped receiving matching events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LossCause {
    /// Its own event or byte bound overflowed.
    QueueOverflow,
    /// It held the largest queue when the session-wide budget ran out.
    SessionBudget,
    /// The fan-out worker fell behind the coordinator and every subscription lost delivery.
    FanoutOverflow,
    /// It was closed while matching events were still undelivered.
    ForcedClose,
}

/// Committed fact that one subscription lost matching events from `from_seq` on.
///
/// It is written to the same durable event log before recovery or close, so a client
/// whose final error frame never arrived reads it on reconnect and resumes from its own
/// last processed cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LossRecord {
    /// Subscription that lost delivery.
    pub subscription_id: SubscriptionId,
    /// First matching sequence the subscription did not deliver.
    pub from_seq: EventSeq,
    /// Why delivery was lost.
    pub cause: LossCause,
}

impl LossRecord {
    /// Records a loss for one subscription.
    pub const fn new(
        subscription_id: SubscriptionId,
        from_seq: EventSeq,
        cause: LossCause,
    ) -> Self {
        Self {
            subscription_id,
            from_seq,
            cause,
        }
    }
}

/// Builds one commit carrying every pending loss; `None` when nothing is pending.
pub(super) fn commit<D: From<LossRecord>>(
    machine: MachineId,
    records: Vec<LossRecord>,
) -> Option<Commit<D>> {
    let affected = || vec![ResourceId::Machine(machine)];
    let mut records = records.into_iter();
    let first = records.next()?;
    let op = OpId::new(uuid::Uuid::now_v7().to_string());
    let commit = Commit::new(op, D::from(first), affected());
    Some(records.fold(commit, |commit, record| {
        commit.event(D::from(record), affected())
    }))
}

//! `events.ack` (IDEAL-022): each subscription keeps the highest sequence delivered to its
//! connection, the highest it examined, and the highest its owner acknowledged. A
//! subscription starts all three at the sequence its starting state reflects. The
//! forwarding task advances the delivered and examined marks before it writes a
//! notification, so the owner can never see an event its acknowledgement would refuse.
//! An acknowledgement past the delivered mark is `seq_ahead`. An older one leaves the
//! acknowledged mark where it is, because the mark never moves backwards.

use std::sync::PoisonError;

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_core::subscriptions::Pulled;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::ids::SubscriptionId;
use ovis_terminal_protocol::schema::methods::events::{EventsAckParams, EventsCursorResult};
use ovis_terminal_server::conn::Call;
use serde_json::{Value, json};

use super::{EventLane, checked};
use crate::assembly::error::refusal;
use crate::assembly::handler::{Served, decode};
use crate::assembly::lanes::ServedEvent;

/// A refused acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum AckError {
    /// The acknowledged sequence was never delivered on this subscription.
    #[error("acknowledged sequence {acknowledged} is past the delivered sequence {delivered}")]
    SeqAhead {
        /// The sequence the owner acknowledged.
        acknowledged: u64,
        /// The highest sequence delivered so far.
        delivered: u64,
    },
}

impl AckError {
    /// The stable error kind.
    pub(super) const fn kind(self) -> &'static str {
        match self {
            Self::SeqAhead { .. } => "seq_ahead",
        }
    }

    /// The recovery hint.
    pub(super) const fn hint(self) -> &'static str {
        match self {
            Self::SeqAhead { .. } => {
                "acknowledge at most the highest sequence delivered on this subscription"
            }
        }
    }
}

/// The delivered, examined and acknowledged marks of one subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Cursor {
    delivered: u64,
    scanned: u64,
    acked: u64,
}

impl Cursor {
    /// A subscription whose starting state reflects every event up to `start`.
    pub(super) const fn at(start: u64) -> Self {
        Self {
            delivered: start,
            scanned: start,
            acked: start,
        }
    }

    /// Raises the delivered and examined marks; neither moves backwards.
    pub(super) fn advance(&mut self, delivered: u64, scanned: u64) {
        self.delivered = self.delivered.max(delivered);
        self.scanned = self.scanned.max(scanned.max(delivered));
    }

    /// Records an acknowledgement of `seq`. Returns the acknowledged and examined marks.
    pub(super) fn ack(&mut self, seq: u64) -> Result<(u64, u64), AckError> {
        if seq > self.delivered {
            return Err(AckError::SeqAhead {
                acknowledged: seq,
                delivered: self.delivered,
            });
        }
        self.acked = self.acked.max(seq);
        Ok((self.acked, self.scanned))
    }
}

/// Who opened a subscription and how far it got.
#[derive(Debug, Clone)]
pub(super) struct Owned {
    pub(super) principal: PrincipalId,
    pub(super) cursor: Cursor,
}

/// The delivered and examined marks one pull reaches, if it delivers anything.
fn reach(
    pulled: &Pulled<ServedEvent, ovis_terminal_core::ops::OperationSnapshot>,
) -> Option<(u64, u64)> {
    match pulled {
        Pulled::Events(batch) => {
            let last = batch
                .events
                .iter()
                .map(|event| event.cursor.seq.get())
                .max()?;
            Some((last, batch.scan_seq.get()))
        }
        Pulled::Heartbeat(beat) => Some((beat.seq.get(), beat.scan_seq.get())),
        _ => None,
    }
}

impl EventLane {
    /// Raises the marks of `id` to what `pulled` delivers, before it is written.
    pub(super) fn advance(
        &self,
        id: &SubscriptionId,
        pulled: &Pulled<ServedEvent, ovis_terminal_core::ops::OperationSnapshot>,
    ) {
        let Some((delivered, scanned)) = reach(pulled) else {
            return;
        };
        let mut owners = self.owners.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(owned) = owners.get_mut(id) {
            owned.cursor.advance(delivered, scanned);
        }
    }

    fn acknowledge(
        &self,
        params: &EventsAckParams,
        principal: &PrincipalId,
    ) -> Result<(u64, u64), BoundaryError> {
        let mut owners = self.owners.lock().unwrap_or_else(PoisonError::into_inner);
        let acked = owners
            .get_mut(&params.subscription_id)
            .filter(|owned| owned.principal == *principal)
            .map(|owned| owned.cursor.ack(params.seq.get()));
        drop(owners);
        acked
            .ok_or_else(|| refusal("not_found", "name a subscription this principal opened"))?
            .map_err(|error| refusal(error.kind(), error.hint()))
    }
}

impl Served {
    /// `events.ack`: records the owner's acknowledgement on its open subscription.
    pub(in crate::assembly) fn ack(
        &self,
        call: &Call,
        principal: &PrincipalId,
    ) -> Result<Value, BoundaryError> {
        let params: EventsAckParams = decode(call)?;
        let (seq, scan_seq) = self.events.acknowledge(&params, principal)?;
        checked::<EventsCursorResult>(json!({
            "seq": seq.to_string(), "scan_seq": scan_seq.to_string()}))
    }
}

#[cfg(test)]
mod tests {
    use super::{AckError, Cursor};

    #[test]
    fn an_acknowledgement_up_to_the_delivered_mark_is_recorded_with_the_scan_mark() {
        let mut cursor = Cursor::at(4);
        cursor.advance(9, 12);
        assert_eq!(cursor.ack(7), Ok((7, 12)));
        assert_eq!(cursor.ack(9), Ok((9, 12)));
    }

    #[test]
    fn an_acknowledgement_past_the_delivered_mark_is_seq_ahead_and_changes_nothing() {
        let mut cursor = Cursor::at(4);
        cursor.advance(9, 12);
        let refused = cursor.ack(10);
        assert_eq!(
            refused,
            Err(AckError::SeqAhead {
                acknowledged: 10,
                delivered: 9
            })
        );
        assert_eq!(refused.map_err(AckError::kind), Err("seq_ahead"));
        assert_eq!(cursor.ack(9), Ok((9, 12)));
    }

    #[test]
    fn the_acknowledged_mark_never_moves_backwards() {
        let mut cursor = Cursor::at(0);
        cursor.advance(20, 20);
        assert_eq!(cursor.ack(15), Ok((15, 20)));
        assert_eq!(cursor.ack(3), Ok((15, 20)));
    }

    #[test]
    fn marks_start_at_the_starting_state_and_never_move_backwards() {
        let mut cursor = Cursor::at(30);
        assert_eq!(cursor.ack(30), Ok((30, 30)));
        cursor.advance(10, 5);
        assert_eq!(cursor.ack(31).map_err(AckError::kind), Err("seq_ahead"));
        cursor.advance(u64::MAX, u64::MAX);
        assert_eq!(cursor.ack(u64::MAX), Ok((u64::MAX, u64::MAX)));
    }
}

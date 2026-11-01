//! Failing calls past their deadline, or whose caller left, and cancelling them remotely.

use std::collections::BTreeMap;

use serde_json::json;
use tokio::time::Instant;

use super::{Actor, ActorError};
use crate::actors::line::Line;

impl<L: Line> Actor<L> {
    /// Fails calls past their deadline, or whose caller left, and tells the peer to cancel.
    ///
    /// Health records the expiries before any caller is answered, so a caller that reads
    /// health after its `deadline_exceeded` always sees its own expiry counted.
    pub(super) async fn expire(&mut self) {
        let now = Instant::now();
        let (removed, kept): (BTreeMap<_, _>, BTreeMap<_, _>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|(_, pending)| pending.due <= now || pending.reply.is_closed());
        self.pending = kept;
        let expired = removed
            .iter()
            .filter(|(_, pending)| pending.due <= now)
            .count();
        if expired > 0 {
            let expired = u64::try_from(expired).unwrap_or(u64::MAX);
            self.health.expired((expired, self.pending.len()));
        } else {
            self.health.in_flight(self.pending.len());
        }
        for (id, pending) in removed {
            if pending.due <= now {
                let machine = self.machine.clone();
                let deadline = pending.deadline;
                drop(
                    pending
                        .reply
                        .send(Err(ActorError::DeadlineExceeded { machine, deadline })),
                );
            }
            let cancel = json!({"jsonrpc": "2.0", "method": "$/cancelRequest",
                "params": {"id": id.to_string()}});
            self.write_control(&cancel).await;
        }
    }
}

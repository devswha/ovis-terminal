//! Failing calls past their deadline, or whose caller left, and cancelling them remotely.

use serde_json::json;
use tokio::time::Instant;

use super::{Actor, ActorError};
use crate::actors::line::Line;

impl<L: Line> Actor<L> {
    /// Fails calls past their deadline, or whose caller left, and tells the peer to cancel.
    pub(super) async fn expire(&mut self) {
        let now = Instant::now();
        let ended: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.due <= now || pending.reply.is_closed())
            .map(|(id, _)| *id)
            .collect();
        let mut expired = 0_u64;
        for id in ended {
            let Some(pending) = self.pending.remove(&id) else {
                continue;
            };
            if pending.due <= now {
                expired = expired.saturating_add(1);
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
        if expired > 0 {
            self.health.expired((expired, self.pending.len()));
        } else {
            self.health.in_flight(self.pending.len());
        }
    }
}

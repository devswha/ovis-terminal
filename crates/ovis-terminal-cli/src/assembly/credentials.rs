//! Credential rotation and revocation (plan todo 57): queued writes are rechecked against
//! the authority on every live terminal, and the ones that no longer hold are fenced with
//! zero bytes before the holder writes them.

use ovis_terminal_core::input::receipts::{Delivery, Receipt};
use ovis_terminal_core::input::sequencer::View;
use ovis_terminal_protocol::domain::credential_rotation::{CommitRace, RacedWrite};
use ovis_terminal_server::dispatch::lanes::credential_rotation::QueuedWrites;

use super::actor::{Actor, Runtime};

/// The runtime's sequencers, as the credential lane rechecks them.
#[derive(Debug)]
pub(super) struct ActorWrites(pub(super) Actor);

fn raced(receipt: &Receipt) -> Option<RacedWrite> {
    let Delivery::Fenced { reason } = &receipt.delivery else {
        return None;
    };
    let reason = serde_json::from_value(serde_json::to_value(reason).ok()?).ok()?;
    let seq = serde_json::from_value(serde_json::to_value(receipt.queued_seq).ok()?).ok()?;
    Some(RacedWrite::new(
        (receipt.op_id.clone(), seq),
        receipt.admitted_bytes,
        CommitRace::CancelledBeforeCommit { reason },
    ))
}

impl Runtime {
    /// Fences every queued write that its admission no longer authorizes.
    fn fence_all(&mut self) -> Vec<RacedWrite> {
        let holder = self.holder.clone();
        let mut settled = Vec::new();
        for live in self.live.values_mut().filter(|live| !live.ended) {
            let Ok((modes, moment)) = live.view_parts() else {
                continue;
            };
            let sequencer = &mut live.sequencer;
            let Ok(Ok(step)) = live
                .hub
                .with_leases(|leases| sequencer.fence(&View::new(leases, &modes, moment)))
            else {
                continue;
            };
            settled.extend(step.fenced.iter().filter_map(raced));
            if let Some(handshake) = step.dispatch
                && let Err(error) = holder.input(handshake)
            {
                drop(super::error::internal("send a fence handshake")(error));
            }
        }
        settled
    }
}

impl QueuedWrites for ActorWrites {
    fn recheck(&self) -> Vec<RacedWrite> {
        self.0.call_blocking(Runtime::fence_all).unwrap_or_default()
    }
}

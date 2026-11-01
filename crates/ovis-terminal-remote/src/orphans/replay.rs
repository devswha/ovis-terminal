//! Sending one machine's pending notices in order, each acknowledged before the next.

use std::time::Duration;

use ovis_terminal_protocol::domain::machine_actor::{
    Delivery, ORPHANED, OrphanedParams, OrphanedResult,
};
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::scalar::{Count, OpenEnum};

use super::{Acknowledgements, OrphanError, Outbox};
use crate::actors::{Actors, Line, Request};

/// What one replay settled.
#[derive(Debug)]
#[non_exhaustive]
pub struct Replayed {
    /// Notices the receiver applied now.
    pub applied: Count,
    /// Notices the receiver had already applied; only their acknowledgement was missing.
    pub duplicates: Count,
    /// Notices still pending after the replay.
    pub remaining: Count,
    /// Why the replay stopped early, if it did; the rest is sent on the next replay.
    pub stopped: Option<OrphanError>,
}

fn echoes(result: &OrphanedResult, params: &OrphanedParams) -> bool {
    result.notification_id == params.notification_id
        && result.parent_incarnation == params.parent_incarnation
}

fn count(value: usize) -> Count {
    Count::new(u64::try_from(value).unwrap_or(u64::MAX))
}

/// Sends `machine`'s pending notices oldest first, each within `deadline`, removing each
/// from the outbox once acknowledged. Replays of one machine never overlap.
///
/// # Errors
///
/// Only an outbox failure before anything was sent; a failed delivery stops the replay
/// and is reported in [`Replayed::stopped`].
pub async fn replay<L: Line, O: Outbox>(
    (actors, outbox): (&Actors<L>, &O),
    acknowledgements: &Acknowledgements,
    (machine, deadline): (&MachineId, Duration),
) -> Result<Replayed, OrphanError> {
    let _turn = acknowledgements.turn(machine).lock_owned().await;
    let pending = outbox.pending(machine).await?;
    let (mut applied, mut duplicates, mut settled) = (0_usize, 0_usize, 0_usize);
    let mut stopped = None;
    for notice in &pending {
        let params = OrphanedParams::to(machine, notice.clone());
        let request = Request::new(ORPHANED, serde_json::to_value(&params)?, deadline);
        let outcome = match actors.call(machine, request).await {
            Ok(Ok(result)) => {
                serde_json::from_value::<OrphanedResult>(result).map_err(OrphanError::from)
            }
            Ok(Err(_)) => Err(OrphanError::Refused),
            Err(error) => Err(error.into()),
        };
        let result = match outcome {
            Ok(result) if echoes(&result, &params) => result,
            Ok(_) => {
                stopped = Some(OrphanError::Mismatch);
                break;
            }
            Err(error) => {
                stopped = Some(error);
                break;
            }
        };
        if let Err(error) = outbox.acknowledge(machine, notice).await {
            stopped = Some(error.into());
            break;
        }
        acknowledgements.settled(&params);
        settled = settled.saturating_add(1);
        match result.delivery {
            OpenEnum::Known(Delivery::Duplicate) => duplicates = duplicates.saturating_add(1),
            _ => applied = applied.saturating_add(1),
        }
    }
    Ok(Replayed {
        applied: count(applied),
        duplicates: count(duplicates),
        remaining: count(pending.len().saturating_sub(settled)),
        stopped,
    })
}

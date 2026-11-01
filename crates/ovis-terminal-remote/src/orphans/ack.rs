//! Settling a pending notice by id, and remembering recently settled ones.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ovis_terminal_protocol::domain::machine_actor::{
    AckOutcome, NotificationId, OrphanedAckParams, OrphanedAckResult, OrphanedParams,
    ParentIncarnation,
};
use ovis_terminal_protocol::schema::ids::MachineId;

use super::{OrphanError, Outbox};

/// Settled notices remembered to answer a repeated acknowledgement as a duplicate.
const REMEMBERED: usize = 1024;

#[derive(Debug, Default)]
struct State {
    settled: BTreeMap<NotificationId, ParentIncarnation>,
    order: VecDeque<NotificationId>,
    turns: BTreeMap<MachineId, Arc<tokio::sync::Mutex<()>>>,
}

/// Acknowledgement bookkeeping shared by every machine's replays.
#[derive(Debug, Default)]
pub struct Acknowledgements(Mutex<State>);

impl Acknowledgements {
    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The lock that keeps replays to `machine` from overlapping.
    pub(super) fn turn(&self, machine: &MachineId) -> Arc<tokio::sync::Mutex<()>> {
        let mut state = self.state();
        let turn = Arc::clone(state.turns.entry(machine.clone()).or_default());
        drop(state);
        turn
    }

    /// Remembers that `params` was settled.
    pub(super) fn settled(&self, params: &OrphanedParams) {
        let mut state = self.state();
        let id = params.notification_id.clone();
        if state
            .settled
            .insert(id.clone(), params.parent_incarnation.clone())
            .is_none()
        {
            state.order.push_back(id);
        }
        while state.order.len() > REMEMBERED {
            if let Some(oldest) = state.order.pop_front() {
                state.settled.remove(&oldest);
            }
        }
        drop(state);
    }

    /// Settles the notice `ack` names among the pending notices of `machines`.
    ///
    /// # Errors
    ///
    /// `not_found` for an id no pending or recently settled notice has; `stale_target`
    /// when the id's notice belongs to another parent incarnation; the outbox failure.
    /// Neither refusal changes the outbox.
    pub async fn acknowledge<O: Outbox>(
        &self,
        (outbox, machines): (&O, &[MachineId]),
        ack: &OrphanedAckParams,
    ) -> Result<OrphanedAckResult, OrphanError> {
        let id = &ack.notification_id;
        for machine in machines {
            let pending = outbox.pending(machine).await?;
            let named = pending
                .into_iter()
                .find(|notice| NotificationId::of(machine, notice) == *id);
            let Some(notice) = named else {
                continue;
            };
            if ParentIncarnation::of(&notice) != ack.parent_incarnation {
                return Err(OrphanError::StaleTarget {
                    notification_id: id.clone(),
                });
            }
            outbox.acknowledge(machine, &notice).await?;
            self.settled(&OrphanedParams::to(machine, notice));
            return Ok(OrphanedAckResult::new(id.clone(), AckOutcome::Acknowledged));
        }
        let settled = self.state().settled.get(id).cloned();
        match settled {
            Some(parent) if parent == ack.parent_incarnation => {
                Ok(OrphanedAckResult::new(id.clone(), AckOutcome::Duplicate))
            }
            Some(_) => Err(OrphanError::StaleTarget {
                notification_id: id.clone(),
            }),
            None => Err(OrphanError::NotFound {
                notification_id: id.clone(),
            }),
        }
    }
}

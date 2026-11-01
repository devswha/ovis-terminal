//! The receiving side: a notice addressed to this machine is applied at most once.

use std::collections::{BTreeSet, VecDeque};
use std::future::Future;
use std::sync::{Mutex, MutexGuard, PoisonError};

use ovis_terminal_protocol::domain::agent_tree::OrphanNotice;
use ovis_terminal_protocol::domain::machine_actor::{
    Delivery, NotificationId, OrphanedParams, OrphanedResult, ParentIncarnation,
};
use ovis_terminal_protocol::schema::ids::MachineId;

use super::{OrphanError, OutboxError};

/// Applied ids remembered in memory; older replays still meet the durable apply's dedupe.
const REMEMBERED: usize = 4096;

#[derive(Debug, Default)]
struct Applied {
    ids: BTreeSet<NotificationId>,
    order: VecDeque<NotificationId>,
}

/// Orphan notices received by one machine.
#[derive(Debug)]
pub struct Inbox {
    machine: MachineId,
    applied: Mutex<Applied>,
}

impl Inbox {
    /// Notices addressed to `machine`.
    #[must_use]
    pub fn new(machine: MachineId) -> Self {
        Self {
            machine,
            applied: Mutex::new(Applied::default()),
        }
    }

    fn applied(&self) -> MutexGuard<'_, Applied> {
        self.applied.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Applies `params` through `apply` unless its id was applied before. `apply` returns
    /// whether it changed anything; a durable replay it recognizes counts as a duplicate.
    ///
    /// # Errors
    ///
    /// `stale_target` for a notice addressed to another machine or carrying another
    /// parent incarnation than it names; the failure of `apply`. Nothing is applied then.
    pub async fn receive<F, A>(
        &self,
        params: &OrphanedParams,
        apply: F,
    ) -> Result<OrphanedResult, OrphanError>
    where
        F: FnOnce(OrphanNotice) -> A,
        A: Future<Output = Result<bool, OutboxError>>,
    {
        let addressed = NotificationId::of(&self.machine, &params.notice) == params.notification_id;
        if !addressed || ParentIncarnation::of(&params.notice) != params.parent_incarnation {
            return Err(OrphanError::StaleTarget {
                notification_id: params.notification_id.clone(),
            });
        }
        let seen = self.applied().ids.contains(&params.notification_id);
        if seen {
            return Ok(OrphanedResult::acknowledging(params, Delivery::Duplicate));
        }
        let changed = apply(params.notice.clone()).await?;
        let mut applied = self.applied();
        if applied.ids.insert(params.notification_id.clone()) {
            applied.order.push_back(params.notification_id.clone());
        }
        while applied.order.len() > REMEMBERED {
            if let Some(oldest) = applied.order.pop_front() {
                applied.ids.remove(&oldest);
            }
        }
        drop(applied);
        let delivery = if changed {
            Delivery::Applied
        } else {
            Delivery::Duplicate
        };
        Ok(OrphanedResult::acknowledging(params, delivery))
    }
}

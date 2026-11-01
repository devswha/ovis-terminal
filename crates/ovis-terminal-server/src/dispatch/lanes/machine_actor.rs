//! Per-machine actors (plan todo 44): `machine.list` with unreachable machines kept, orphan
//! notices replayed to a machine when its line returns, and `$/orphanedAck` settling one.
//!
//! The roster is read from each actor's published state without messaging any actor, so
//! a stalled machine never delays it. Orphan notices come from the agent tree's durable
//! outbox (plan todo 41) and leave it only when the receiving machine acknowledges them.

mod outbox;
mod pump;

use std::sync::Arc;
use std::time::Duration;

pub use outbox::TreeOutbox;
use ovis_terminal_agent::status::{AgentStatusEvent, ProcessOwner};
use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::registry::MachineId as LocalMachine;
use ovis_terminal_protocol::domain::machine_actor::{
    NotificationId, OrphanedAckParams, OrphanedParams, OrphanedResult,
};
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::schema::common::NoParams;
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::methods::machine::MachineListResult;
use ovis_terminal_remote::actors::{ActorError, Actors, Line};
use ovis_terminal_remote::orphans::{
    Acknowledgements, Inbox, OrphanError, Outbox as _, Replayed, replay,
};
use ovis_terminal_store::Snapshot;
pub use pump::{PumpReport, pump};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::dispatch::{Access, DispatchError, Read};

/// The typed refusal of a remote-actor failure.
fn refusal((kind, hint): (&'static str, &'static str), retryable: bool) -> BoundaryError {
    BoundaryError::domain(ErrorKind::from(kind.to_owned()), retryable).with_hint(hint)
}

/// The boundary form of an orphan failure.
#[must_use]
pub fn orphan_refusal(error: &OrphanError) -> BoundaryError {
    let retryable = matches!(error, OrphanError::Actor(actor) if actor.retryable());
    refusal((error.kind(), error.hint()), retryable)
}

/// The boundary form of an actor failure.
#[must_use]
pub fn actor_refusal(error: &ActorError) -> BoundaryError {
    refusal((error.kind(), error.hint()), error.retryable())
}

/// Machine actors of this runtime and the orphan notices owed to them.
#[derive(Debug)]
pub struct MachineActorLane<L, D, P> {
    local: LocalMachine,
    actors: Arc<Actors<L>>,
    outbox: TreeOutbox<D, P>,
    acknowledgements: Acknowledgements,
    inbox: Inbox,
    delivery_deadline: Duration,
}

impl<L, D, P> MachineActorLane<L, D, P>
where
    L: Line,
    D: EventData
        + From<AgentStatusEvent>
        + Clone
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
{
    /// The lane of the runtime on `local` over `actors`, delivering orphan notices from
    /// `outbox`, each within `delivery_deadline`.
    #[must_use]
    pub fn new(
        local: LocalMachine,
        (actors, outbox): (Arc<Actors<L>>, TreeOutbox<D, P>),
        delivery_deadline: Duration,
    ) -> Arc<Self> {
        let inbox = Inbox::new(MachineId::new(local.to_string()));
        Arc::new(Self {
            local,
            actors,
            outbox,
            acknowledgements: Acknowledgements::default(),
            inbox,
            delivery_deadline,
        })
    }

    /// The machine actors.
    #[must_use]
    pub const fn actors(&self) -> &Arc<Actors<L>> {
        &self.actors
    }

    /// What acknowledging an orphan notice needs: administering this runtime's machines.
    #[must_use]
    pub const fn acknowledge_access(&self) -> Access {
        Access::new(
            Action::Administer,
            ResolvedTarget::new(self.local, TargetResource::Machine),
        )
    }

    /// Hands `line` to `machine`'s actor, then replays the notices it has not acknowledged.
    ///
    /// # Errors
    ///
    /// The actor or outbox failure; a replay that stops early is reported in the result.
    pub async fn attach(&self, machine: &MachineId, line: L) -> Result<Replayed, OrphanError> {
        self.actors.attach(machine, line).await?;
        self.deliver(machine).await
    }

    /// Replays `machine`'s pending orphan notices, oldest first.
    ///
    /// # Errors
    ///
    /// The outbox failure before anything was sent.
    pub async fn deliver(&self, machine: &MachineId) -> Result<Replayed, OrphanError> {
        let target = (machine, self.delivery_deadline);
        replay(
            (&*self.actors, &self.outbox),
            &self.acknowledgements,
            target,
        )
        .await
    }

    /// Ids of the orphan notices `machine` has not acknowledged, oldest first.
    ///
    /// # Errors
    ///
    /// The outbox failure.
    pub async fn pending(&self, machine: &MachineId) -> Result<Vec<NotificationId>, OrphanError> {
        let pending = self.outbox.pending(machine).await?;
        Ok(pending
            .iter()
            .map(|notice| NotificationId::of(machine, notice))
            .collect())
    }

    /// `$/orphanedAck`: settles the pending notice the params name.
    ///
    /// # Errors
    ///
    /// `invalid_params`, `not_found` or `stale_target`, leaving the outbox unchanged.
    pub async fn acknowledge(&self, params: Value) -> Result<Value, BoundaryError> {
        let params: OrphanedAckParams = serde_json::from_value(params)
            .map_err(|_| BoundaryError::invalid_params(Some("params")))?;
        let machines = self.actors.machine_ids();
        let settled = self
            .acknowledgements
            .acknowledge((&self.outbox, &machines), &params)
            .await
            .map_err(|error| orphan_refusal(&error))?;
        serde_json::to_value(settled)
            .map_err(|_| refusal(("internal", "retry the acknowledgement"), true))
    }

    /// `$/orphaned` received from the parent's machine: children here that opted to die
    /// with that exact incarnation are returned as end orders, once per notice.
    ///
    /// # Errors
    ///
    /// `stale_target` for a notice addressed elsewhere; the agent tree's failure.
    pub async fn orphaned(
        &self,
        params: &OrphanedParams,
    ) -> Result<(OrphanedResult, Vec<ProcessOwner>), OrphanError> {
        let mut orders = Vec::new();
        let result = self
            .inbox
            .receive(params, |notice| async {
                let exit = self.outbox.apply(notice).await?;
                let applied = !exit.replayed;
                orders = exit.end_orders;
                Ok(applied)
            })
            .await?;
        Ok((result, orders))
    }
}

/// `machine.list`: every machine with its line state; an unreachable one stays listed.
#[derive(Debug)]
pub struct MachineList<L, D, P>(Arc<MachineActorLane<L, D, P>>);

impl<L, D, P> MachineList<L, D, P> {
    /// The roster of `lane`.
    #[must_use]
    pub fn new(lane: &Arc<MachineActorLane<L, D, P>>) -> Self {
        Self(Arc::clone(lane))
    }
}

impl<L, D, P> Read for MachineList<L, D, P>
where
    L: Line + std::fmt::Debug,
    D: EventData
        + From<AgentStatusEvent>
        + Clone
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + std::fmt::Debug
        + 'static,
    P: SnapshotProducer + Send + Sync + std::fmt::Debug + 'static,
{
    const NAME: &'static str = "machine.list";
    type Params = NoParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        let target = ResolvedTarget::new(self.0.local, TargetResource::Machine);
        Ok(Access::new(Action::Read, target))
    }

    fn read(&self, _: Self::Params, _: &Snapshot) -> Result<Value, BoundaryError> {
        let roster = MachineListResult::new(self.0.actors.list());
        serde_json::to_value(roster).map_err(|_| refusal(("internal", "retry the read"), true))
    }
}

#[cfg(all(test, unix))]
mod qa;

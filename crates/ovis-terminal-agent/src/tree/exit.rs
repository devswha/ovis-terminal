//! Program exits and received parent deaths.
//!
//! One unit records the finish, orders the end of the opted-in children of that exact
//! incarnation, queues the death for every machine holding its children, ends the
//! program's own link with a notice when its parent asked for one, and expires the
//! program's open prompts. A replayed finish changes nothing and orders nothing.

use ovis_terminal_core::events::{Commit, Coordinator, EventData, ResourceId, SnapshotProducer};
use ovis_terminal_protocol::domain::agent_tree::{
    ChildNotice, OrphanNotice, ParentDeath, RemoteDelivery,
};
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::records::ParentRef;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_store::{CasWrite, OpId, Snapshot};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::service::{child_ref, owner_of, wire};
use super::store::{self, ParentKey};
use super::{TreeError, TreeService};
use crate::status::{AgentStatusEvent, ProcessOwner};

/// What one exit or received death changed.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct Exit {
    /// The finish was already recorded; nothing changed and nothing was ordered.
    pub replayed: bool,
    /// Children of this machine to end; each counts as ended only once its exit is seen.
    pub end_orders: Vec<ProcessOwner>,
    /// Deaths to send now to machines holding children, kept until they acknowledge.
    pub deliveries: Vec<(MachineId, OrphanNotice)>,
}

/// Sequence of the unit's event at 1-based `position`.
pub(super) const fn event_seq(snapshot: &Snapshot, position: u64) -> EventSeq {
    EventSeq::new(snapshot.revision().get().saturating_add(position))
}

/// An event and the resources it affects.
pub(super) type Change = (AgentStatusEvent, Vec<ResourceId>);

/// One commit of `first` then `rest`, in order, with `writes`.
pub(super) fn commit<D>(
    (first, affected): Change,
    rest: Vec<Change>,
    writes: Vec<CasWrite>,
) -> Commit<D>
where
    D: From<AgentStatusEvent>,
{
    let op = OpId::new(uuid::Uuid::now_v7().to_string());
    let mut unit = Commit::new(op, D::from(first), affected);
    for (event, affected) in rest {
        unit = unit.event(D::from(event), affected);
    }
    for write in writes {
        unit = unit.write(write);
    }
    unit
}

fn settled(outcome: Result<Exit, TreeError>) -> Result<Exit, TreeError> {
    match outcome {
        Err(TreeError::AlreadyFinished) => Ok(Exit {
            replayed: true,
            ..Exit::default()
        }),
        other => other,
    }
}

impl TreeService {
    /// Records `key`'s finish at `seq` once and orders the end of its opted-in live children.
    fn finish(
        &self,
        snapshot: &Snapshot,
        (key, seq): (&ParentKey, EventSeq),
    ) -> Result<(ParentDeath, Exit, Vec<CasWrite>), TreeError> {
        let mut finished = store::finished(snapshot, (self.runtime, key))?;
        if finished.covers(key.parent.incarnation) {
            return Err(TreeError::AlreadyFinished);
        }
        finished.value.through = key.parent.incarnation;
        let mut writes = vec![finished.write()?];
        let mut row = store::parent(snapshot, (self.runtime, key))?;
        let mut death = ParentDeath::new(key.parent.clone(), key.machine.clone(), seq);
        let mut exit = Exit::default();
        for link in &mut row.value.links {
            if link.order_end() {
                death.ending.push(link.child.clone());
                exit.end_orders.extend(owner_of(&link.child));
            } else if link.is_running() {
                death.kept.push(link.child.clone());
            }
        }
        for machine in &row.value.remote {
            death.remote.push(RemoteDelivery::pending(machine.clone()));
            let mut outbox = store::outbox(snapshot, (self.runtime, machine))?;
            outbox.value.notices.push(death.notice());
            writes.push(outbox.write()?);
            exit.deliveries.push((machine.clone(), death.notice()));
        }
        if row.present {
            row.value.death = Some(death.clone());
            writes.push(row.write()?);
        }
        Ok((death, exit, writes))
    }

    /// Records the observed exit of `owner`'s program.
    ///
    /// # Errors
    ///
    /// Returns a store, decoding or coordinator failure; a replayed exit is not an error.
    pub fn exited<D, P>(
        &self,
        coordinator: &Coordinator<D, P>,
        owner: ProcessOwner,
    ) -> Result<Exit, TreeError>
    where
        D: EventData + From<AgentStatusEvent> + Clone + Serialize + DeserializeOwned,
        P: SnapshotProducer,
    {
        let mut outcome = Exit::default();
        let committed = coordinator.commit_unit(|snapshot| {
            let parent = ParentRef::new(wire(owner.terminal), owner.process);
            let local = self.local(parent);
            let (death, exit, mut writes) =
                self.finish(snapshot, (&local, event_seq(snapshot, 1)))?;
            let affected = vec![ResourceId::Terminal(owner.terminal)];
            let finished = AgentStatusEvent::ParentFinished(Box::new(death));
            let mut events = Vec::new();
            let ended = self.end_link(snapshot, (owner, event_seq(snapshot, 2)))?;
            if let Some((event, write)) = ended {
                events.push((event, affected.clone()));
                writes.push(write);
            }
            let position = u64::try_from(events.len()).unwrap_or(u64::MAX);
            let first = event_seq(snapshot, position.saturating_add(2));
            let (closed, expired) = self.prompts.expire_program(snapshot, (owner, first))?;
            for record in closed {
                let event = AgentStatusEvent::PromptClosed(Box::new(record));
                events.push((event, affected.clone()));
            }
            writes.extend(expired);
            outcome = exit;
            Ok(commit((finished, affected), events, writes))
        });
        settled(committed.map(|_| outcome))
    }

    /// Ends `owner`'s own link at `seq`, with a notice when its parent asked for news.
    fn end_link(
        &self,
        snapshot: &Snapshot,
        (owner, seq): (ProcessOwner, EventSeq),
    ) -> Result<Option<(AgentStatusEvent, CasWrite)>, TreeError> {
        let child = child_ref(owner);
        let Some(index) = store::child(snapshot, (self.runtime, &child))?.value else {
            return Ok(None);
        };
        let mut row = store::parent(snapshot, (self.runtime, &index.key()))?;
        let Some(link) = row.value.links.iter_mut().find(|link| link.child == child) else {
            return Ok(None);
        };
        let Some(reason) = link.exited(seq) else {
            return Ok(None);
        };
        let notify = link.policy.notify;
        let event = AgentStatusEvent::ChildEnded(Box::new(link.clone()));
        if notify {
            let notice = ChildNotice::new(child, reason.into(), seq);
            row.value.notices.push(notice);
        }
        Ok(Some((event, row.write()?)))
    }

    /// Applies a parent's death received from the machine it ran on: the children held
    /// here that opted to die with that exact incarnation are ordered to end.
    ///
    /// # Errors
    ///
    /// Returns a store, decoding or coordinator failure; a replayed notice is not an error.
    pub fn orphaned<D, P>(
        &self,
        coordinator: &Coordinator<D, P>,
        notice: &OrphanNotice,
    ) -> Result<Exit, TreeError>
    where
        D: EventData + From<AgentStatusEvent> + Clone + Serialize + DeserializeOwned,
        P: SnapshotProducer,
    {
        let key = ParentKey {
            machine: notice.parent_machine.clone(),
            parent: notice.parent.clone(),
        };
        let mut outcome = Exit::default();
        let committed = coordinator.commit_unit(|snapshot| {
            let (death, exit, writes) = self.finish(snapshot, (&key, notice.finished_seq))?;
            let affected = vec![ResourceId::Machine(self.identity.machine_id())];
            let finished = AgentStatusEvent::ParentFinished(Box::new(death));
            outcome = exit;
            Ok(commit((finished, affected), Vec::new(), writes))
        });
        settled(committed.map(|_| outcome))
    }
}

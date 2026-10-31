//! Children on other machines, the deaths they still owe an acknowledgement for, and
//! the parent's acknowledgement of child notices.

use ovis_terminal_core::events::{Coordinator, EventData, ResourceId, SnapshotProducer};
use ovis_terminal_protocol::domain::agent_tree::{NoticeAck, OrphanAck, OrphanNotice};
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::records::ParentRef;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_store::{CasWrite, Snapshot};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::exit::{commit, event_seq};
use super::store::{self, ParentKey};
use super::{Link, Linked, TreeError, TreeService};
use crate::status::AgentStatusEvent;

impl TreeService {
    /// Links a child launched here to a parent incarnation on `parent_machine`, unless
    /// that machine's death of the incarnation was already received.
    ///
    /// # Errors
    ///
    /// Returns `parent_finished`, `limit_exceeded`, or a store or decoding failure.
    pub fn link_to_remote(
        &self,
        snapshot: &Snapshot,
        (parent_machine, link): (MachineId, Link),
    ) -> Result<Linked, TreeError> {
        let key = ParentKey {
            machine: parent_machine,
            parent: link.parent.clone(),
        };
        if store::finished(snapshot, (self.runtime, &key))?.covers(key.parent.incarnation) {
            return Err(TreeError::ParentFinished);
        }
        self.record(snapshot, (key, link, None))
    }

    /// Records, in the launch's unit, that `machine` holds a child of the live `parent`,
    /// so the parent's death is queued for it.
    ///
    /// # Errors
    ///
    /// Returns the [`admit`](Self::admit) refusals, or a store or decoding failure.
    pub fn link_remote(
        &self,
        snapshot: &Snapshot,
        (parent, machine): (ParentRef, MachineId),
    ) -> Result<Vec<CasWrite>, TreeError> {
        self.admit(snapshot, &parent)?;
        let mut row = store::parent(snapshot, (self.runtime, &self.local(parent)))?;
        if !row.value.remote.contains(&machine) {
            row.value.remote.push(machine);
        }
        Ok(vec![row.write()?])
    }

    /// Deaths `machine` has not acknowledged, oldest first, to send again after it
    /// reconnects.
    ///
    /// # Errors
    ///
    /// Returns a store or decoding failure.
    pub fn outbox(
        &self,
        snapshot: &Snapshot,
        machine: &MachineId,
    ) -> Result<Vec<OrphanNotice>, TreeError> {
        Ok(store::outbox(snapshot, (self.runtime, machine))?
            .value
            .notices)
    }

    /// Records `ack`; returns whether it acknowledged a pending delivery. A replayed
    /// acknowledgement changes nothing.
    ///
    /// # Errors
    ///
    /// Returns a store, decoding or coordinator failure.
    pub fn orphan_acked<D, P>(
        &self,
        coordinator: &Coordinator<D, P>,
        ack: &OrphanAck,
    ) -> Result<bool, TreeError>
    where
        D: EventData + From<AgentStatusEvent> + Clone + Serialize + DeserializeOwned,
        P: SnapshotProducer,
    {
        let committed = coordinator.commit_unit(|snapshot| {
            let mut outbox = store::outbox(snapshot, (self.runtime, &ack.machine_id))?;
            let pending = outbox.value.notices.len();
            outbox.value.notices.retain(|notice| notice != &ack.notice);
            let mut writes = Vec::new();
            if outbox.value.notices.len() != pending {
                writes.push(outbox.write()?);
            }
            let key = ParentKey {
                machine: ack.notice.parent_machine.clone(),
                parent: ack.notice.parent.clone(),
            };
            let mut row = store::parent(snapshot, (self.runtime, &key))?;
            let delivered = row.value.death.as_mut();
            if delivered.is_some_and(|death| death.acknowledge(&ack.machine_id)) {
                writes.push(row.write()?);
            }
            if writes.is_empty() {
                return Err(TreeError::AlreadyAcknowledged);
            }
            let affected = vec![ResourceId::Machine(self.identity.machine_id())];
            let event = AgentStatusEvent::OrphanAcknowledged(Box::new(ack.clone()));
            Ok(commit((event, affected), Vec::new(), writes))
        });
        match committed {
            Ok(_) => Ok(true),
            Err(TreeError::AlreadyAcknowledged) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Acknowledges, for `parent`, every notice about its children through `through`.
    ///
    /// # Errors
    ///
    /// Returns a store, decoding or coordinator failure.
    pub fn acknowledge<D, P>(
        &self,
        coordinator: &Coordinator<D, P>,
        (parent, through): (ParentRef, EventSeq),
    ) -> Result<NoticeAck, TreeError>
    where
        D: EventData + From<AgentStatusEvent> + Clone + Serialize + DeserializeOwned,
        P: SnapshotProducer,
    {
        let mut acknowledged = Vec::new();
        let committed = coordinator.commit_unit(|snapshot| {
            let mut row = store::parent(snapshot, (self.runtime, &self.local(parent.clone())))?;
            let seq = event_seq(snapshot, 1);
            for notice in &mut row.value.notices {
                if notice.notice_seq <= through && notice.acknowledged_seq.is_none() {
                    notice.acknowledged_seq = Some(seq);
                    acknowledged.push(notice.notice_seq);
                }
            }
            if acknowledged.is_empty() {
                return Err(TreeError::AlreadyAcknowledged);
            }
            let ack = NoticeAck::new(parent.clone(), through, acknowledged.clone());
            let affected = vec![ResourceId::Machine(self.identity.machine_id())];
            let event = AgentStatusEvent::NoticesAcknowledged(Box::new(ack));
            Ok(commit((event, affected), Vec::new(), vec![row.write()?]))
        });
        match committed {
            Ok(_) | Err(TreeError::AlreadyAcknowledged) => {
                Ok(NoticeAck::new(parent, through, acknowledged))
            }
            Err(error) => Err(error),
        }
    }
}

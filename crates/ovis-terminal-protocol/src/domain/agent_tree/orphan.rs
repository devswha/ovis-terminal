//! A parent's death as a durable fact and its delivery to the machines holding its
//! children, kept until each acknowledges it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ChildRef;
use crate::schema::ids::MachineId;
use crate::schema::records::ParentRef;
use crate::schema::scalar::EventSeq;

/// Delivery of a parent's death to one machine holding its children.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RemoteDelivery {
    /// Machine holding children of the parent.
    pub machine_id: MachineId,
    /// Whether that machine acknowledged the death.
    pub acknowledged: bool,
}

/// The durable fact that one parent program incarnation finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ParentDeath {
    /// The finished parent program.
    pub parent: ParentRef,
    /// Machine the parent ran on.
    pub parent_machine: MachineId,
    /// Sequence of the event that recorded the death.
    pub finished_seq: EventSeq,
    /// Children of this machine ordered to end with the parent.
    pub ending: Vec<ChildRef>,
    /// Children of this machine that keep running.
    pub kept: Vec<ChildRef>,
    /// Other machines holding children, each awaiting acknowledgement.
    pub remote: Vec<RemoteDelivery>,
}

impl RemoteDelivery {
    /// A delivery to `machine_id` that awaits its acknowledgement.
    #[must_use]
    pub const fn pending(machine_id: MachineId) -> Self {
        Self {
            machine_id,
            acknowledged: false,
        }
    }
}

impl ParentDeath {
    /// `parent` on `parent_machine` finished at `finished_seq`; no child is listed yet.
    #[must_use]
    pub const fn new(parent: ParentRef, parent_machine: MachineId, finished_seq: EventSeq) -> Self {
        Self {
            parent,
            parent_machine,
            finished_seq,
            ending: Vec::new(),
            kept: Vec::new(),
            remote: Vec::new(),
        }
    }

    /// The notice another machine holding children of this parent receives.
    #[must_use]
    pub fn notice(&self) -> OrphanNotice {
        OrphanNotice {
            parent_machine: self.parent_machine.clone(),
            parent: self.parent.clone(),
            finished_seq: self.finished_seq,
        }
    }

    /// Marks `machine`'s delivery acknowledged; returns whether it was pending.
    pub fn acknowledge(&mut self, machine: &MachineId) -> bool {
        self.remote
            .iter_mut()
            .find(|delivery| &delivery.machine_id == machine && !delivery.acknowledged)
            .is_some_and(|delivery| {
                delivery.acknowledged = true;
                true
            })
    }
}

/// A parent's death, sent to a machine holding its children and replayed until acknowledged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct OrphanNotice {
    /// Machine the parent ran on.
    pub parent_machine: MachineId,
    /// The finished parent program, by incarnation.
    pub parent: ParentRef,
    /// Sequence of the death on the parent's machine.
    pub finished_seq: EventSeq,
}

/// A machine's acknowledgement that it applied an orphan notice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct OrphanAck {
    /// The notice applied.
    pub notice: OrphanNotice,
    /// The acknowledging machine.
    pub machine_id: MachineId,
}

impl OrphanAck {
    /// `machine_id` applied `notice`.
    #[must_use]
    pub const fn new(notice: OrphanNotice, machine_id: MachineId) -> Self {
        Self { notice, machine_id }
    }
}

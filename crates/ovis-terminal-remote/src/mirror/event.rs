//! Mirror records as local events, each committed with its remote origin.

use ovis_terminal_core::events::{Commit, EventData, RemoteOrigin, RemoteSeq, ResourceId};
use ovis_terminal_core::registry::{MachineId as CoreMachineId, RuntimeId};
use ovis_terminal_protocol::domain::machine_actor::{MirrorBoundary, MirroredEvent};
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_store::OpId;
use serde::{Deserialize, Serialize};

use super::MirrorError;

/// A local event produced by mirroring a machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "record", rename_all = "snake_case")]
#[non_exhaustive]
pub enum MirrorEvent {
    /// A remote event.
    Mirrored(Box<MirroredEvent>),
    /// A scan or loss boundary.
    Boundary(Box<MirrorBoundary>),
}

impl EventData for MirrorEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Mirrored(_) => "machine.mirrored",
            Self::Boundary(_) => "machine.boundary",
        }
    }
}

/// One mirror record ready to commit in the local sequence domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mirrored(MirrorEvent);

impl Mirrored {
    pub(super) fn event(event: MirroredEvent) -> Self {
        Self(MirrorEvent::Mirrored(Box::new(event)))
    }

    pub(super) fn boundary(boundary: MirrorBoundary) -> Self {
        Self(MirrorEvent::Boundary(Box::new(boundary)))
    }

    /// The record.
    #[must_use]
    pub const fn record(&self) -> &MirrorEvent {
        &self.0
    }

    fn machine(&self) -> &MachineId {
        match &self.0 {
            MirrorEvent::Mirrored(event) => &event.source.machine_id,
            MirrorEvent::Boundary(boundary) => &boundary.machine_id,
        }
    }

    /// One local commit of this record affecting its machine; a remote event keeps its
    /// machine, lifetime and sequence as the commit's origin.
    ///
    /// # Errors
    ///
    /// [`MirrorError::Identity`] when the machine or lifetime is not a runtime identity.
    pub fn into_commit<D: From<MirrorEvent>>(self) -> Result<Commit<D>, MirrorError> {
        let machine: CoreMachineId = self
            .machine()
            .as_str()
            .parse()
            .map_err(MirrorError::Identity)?;
        let origin = match &self.0 {
            MirrorEvent::Mirrored(event) => {
                let lifetime: RuntimeId = event
                    .source
                    .runtime_lifetime
                    .as_str()
                    .parse()
                    .map_err(MirrorError::Identity)?;
                Some(RemoteOrigin::new(
                    machine,
                    lifetime,
                    RemoteSeq::new(event.source.seq),
                ))
            }
            MirrorEvent::Boundary(_) => None,
        };
        let op = OpId::new(uuid::Uuid::now_v7().to_string());
        let commit = Commit::new(op, D::from(self.0), vec![ResourceId::Machine(machine)]);
        Ok(match origin {
            Some(origin) => commit.mirrored_from(origin),
            None => commit,
        })
    }
}

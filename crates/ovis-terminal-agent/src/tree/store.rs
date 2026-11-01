//! Persisted tree rows, read and written in commit units.
//!
//! A parent row is keyed by the parent's machine, terminal and program incarnation and
//! lives on the machine of the children it holds. Each terminal keeps the highest
//! finished incarnation, so a finish is recorded once and every older incarnation counts
//! as finished. Each peer machine has an outbox of deaths it has not acknowledged.

use ovis_terminal_core::events::EventError;
use ovis_terminal_core::registry::RuntimeId;
use ovis_terminal_protocol::domain::agent_tree::{
    ChildLink, ChildNotice, ChildRef, OrphanNotice, ParentDeath,
};
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::records::ParentRef;
use ovis_terminal_protocol::schema::scalar::Incarnation;
use ovis_terminal_store::{CasWrite, Payload, ResourceKey, ResourceLookup, Snapshot, Version};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::TreeError;

/// Most running children one parent incarnation keeps.
pub(super) const CHILD_LIMIT: usize = 64;

/// A value and the version its compare-and-swap write must expect.
#[derive(Debug)]
pub(super) struct Versioned<T> {
    pub value: T,
    pub present: bool,
    version: Version,
    key: ResourceKey,
}

impl<T: Serialize> Versioned<T> {
    pub(super) fn write(&self) -> Result<CasWrite, TreeError> {
        let bytes = serde_json::to_vec(&self.value)?;
        Ok(CasWrite::new(
            self.key.clone(),
            self.version,
            Some(Payload::from(bytes)),
        ))
    }
}

fn read<T: DeserializeOwned>(
    snapshot: &Snapshot,
    key: ResourceKey,
) -> Result<Versioned<Option<T>>, TreeError> {
    let (value, version) = match snapshot.resource(&key).map_err(EventError::from)? {
        ResourceLookup::Live { value, version, .. } => {
            (Some(serde_json::from_slice(value.as_bytes())?), version)
        }
        ResourceLookup::Tombstone { version, .. } => (None, version),
        ResourceLookup::Absent => (None, Version::ABSENT),
        _ => return Err(EventError::SnapshotUnavailable.into()),
    };
    Ok(Versioned {
        present: value.is_some(),
        value,
        version,
        key,
    })
}

impl<T> Versioned<Option<T>> {
    /// The stored value, or `T::default()` where none is stored.
    fn or_default(self) -> Versioned<T>
    where
        T: Default,
    {
        self.with(Option::unwrap_or_default)
    }

    /// The stored value replaced by `value`, written at the same expected version.
    pub(super) fn with(self, value: impl FnOnce(Option<T>) -> T) -> Versioned<T> {
        Versioned {
            value: value(self.value),
            present: self.present,
            version: self.version,
            key: self.key,
        }
    }
}

/// A parent program incarnation on a machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParentKey {
    pub machine: MachineId,
    pub parent: ParentRef,
}

/// Children held on this machine for one parent incarnation, their news and its death.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ParentRow {
    pub links: Vec<ChildLink>,
    pub notices: Vec<ChildNotice>,
    pub death: Option<ParentDeath>,
    /// Other machines holding children of this parent.
    pub remote: Vec<MachineId>,
}

pub(super) fn parent(
    snapshot: &Snapshot,
    (runtime, key): (RuntimeId, &ParentKey),
) -> Result<Versioned<ParentRow>, TreeError> {
    let name = format!(
        "{runtime}/{}/{}/{}",
        key.machine.as_str(),
        key.parent.terminal_id.as_str(),
        key.parent.incarnation.get()
    );
    read(snapshot, ResourceKey::new("agent_tree_parent", name)).map(Versioned::or_default)
}

/// Where a linked child's parent row lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ChildRow {
    pub parent_machine: MachineId,
    pub parent: ParentRef,
}

impl ChildRow {
    pub(super) fn key(&self) -> ParentKey {
        ParentKey {
            machine: self.parent_machine.clone(),
            parent: self.parent.clone(),
        }
    }
}

pub(super) fn child(
    snapshot: &Snapshot,
    (runtime, child): (RuntimeId, &ChildRef),
) -> Result<Versioned<Option<ChildRow>>, TreeError> {
    let name = format!(
        "{runtime}/{}/{}",
        child.terminal_id.as_str(),
        child.process_incarnation.get()
    );
    read(snapshot, ResourceKey::new("agent_tree_child", name))
}

/// The highest finished program incarnation of one terminal on one machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Finished {
    pub through: Incarnation,
}

impl Default for Finished {
    fn default() -> Self {
        Self {
            through: Incarnation::new(0),
        }
    }
}

impl Versioned<Finished> {
    /// Whether `incarnation` already finished.
    pub(super) fn covers(&self, incarnation: Incarnation) -> bool {
        self.present && incarnation <= self.value.through
    }
}

pub(super) fn finished(
    snapshot: &Snapshot,
    (runtime, key): (RuntimeId, &ParentKey),
) -> Result<Versioned<Finished>, TreeError> {
    let name = format!(
        "{runtime}/{}/{}",
        key.machine.as_str(),
        key.parent.terminal_id.as_str()
    );
    read(snapshot, ResourceKey::new("agent_tree_finished", name)).map(Versioned::or_default)
}

/// Deaths one peer machine has not acknowledged, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Outbox {
    pub notices: Vec<OrphanNotice>,
}

pub(super) fn outbox(
    snapshot: &Snapshot,
    (runtime, machine): (RuntimeId, &MachineId),
) -> Result<Versioned<Outbox>, TreeError> {
    let name = format!("{runtime}/{}", machine.as_str());
    read(snapshot, ResourceKey::new("agent_tree_outbox", name)).map(Versioned::or_default)
}

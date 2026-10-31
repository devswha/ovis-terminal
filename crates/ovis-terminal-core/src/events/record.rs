//! Typed durable event bodies and their commit-order envelopes.

use ovis_terminal_protocol::schema::ids::{AgentId, DocName, ServiceLabel, SubscriptionId};
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_store::{CasWrite, OpId};
use serde::{Deserialize, Serialize};

use crate::registry::{
    MachineId, PaneId, RuntimeId, RuntimeIdentity, ServerBootId, SplitId, TabId, TerminalId,
    WorkspaceId,
};

pub(super) const SCHEMA_VERSION: u32 = 1;

/// One position in a machine's runtime lifetime, independent of control-core boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Cursor {
    /// Machine owning this sequence.
    pub machine_id: MachineId,
    /// Runtime lifetime owning this sequence.
    pub runtime_id: RuntimeId,
    /// Last committed event observed.
    pub seq: EventSeq,
}

impl Cursor {
    /// Combines machine, lifetime and sequence without conflating their domains.
    pub const fn new(machine_id: MachineId, runtime_id: RuntimeId, seq: EventSeq) -> Self {
        Self {
            machine_id,
            runtime_id,
            seq,
        }
    }

    /// Parses a cursor once at the JSON boundary, rejecting numeric counter forms.
    ///
    /// # Errors
    ///
    /// Returns `invalid_params` for malformed identities or noncanonical decimal sequences.
    pub fn decode(bytes: &[u8]) -> Result<Self, super::EventError> {
        serde_json::from_slice(bytes).map_err(super::EventError::InvalidParams)
    }
}

/// An upstream sequence, never substituted for the local mirror sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteSeq(EventSeq);

impl RemoteSeq {
    /// Tags the originating runtime's counter as remote ordering evidence.
    pub const fn new(seq: EventSeq) -> Self {
        Self(seq)
    }

    /// Reads the originating counter without substituting it for a local sequence.
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Original remote identity and ordering evidence retained by a mirrored event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RemoteOrigin {
    /// Originating machine.
    pub machine_id: MachineId,
    /// Originating runtime lifetime.
    pub runtime_id: RuntimeId,
    /// Sequence assigned by that remote runtime.
    pub seq: RemoteSeq,
}

impl RemoteOrigin {
    /// Preserves the source identity together with its distinct sequence.
    pub const fn new(machine_id: MachineId, runtime_id: RuntimeId, seq: RemoteSeq) -> Self {
        Self {
            machine_id,
            runtime_id,
            seq,
        }
    }
}

/// Typed identities of every resource affected by a mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ResourceId {
    /// A workspace.
    Workspace(WorkspaceId),
    /// A tab.
    Tab(TabId),
    /// A pane.
    Pane(PaneId),
    /// A layout split.
    Split(SplitId),
    /// A terminal.
    Terminal(TerminalId),
    /// An agent binding.
    Agent(AgentId),
    /// A machine record.
    Machine(MachineId),
    /// A service registration, distinct from its host machine.
    Service(ServiceLabel),
    /// A document.
    Document(DocName),
}

/// A producer's closed data enum supplies its event name without a parallel parser.
pub trait EventData {
    /// Name of the variant being committed.
    fn event_type(&self) -> &'static str;

    /// Stamps embedded revision fields while the coordinator owns the commit barrier.
    ///
    /// Data without embedded sequence fields needs no adjustment. Composite event
    /// enums must forward this call to variants that carry such fields.
    fn at_sequence(&mut self, _seq: EventSeq) {}
}

/// A mutation and its mandatory typed event, committed in one store transaction.
#[derive(Debug)]
pub struct Commit<D> {
    pub(super) op: OpId,
    pub(super) data: D,
    pub(super) affected: Vec<ResourceId>,
    pub(super) writes: Vec<CasWrite>,
    pub(super) origin: Option<RemoteOrigin>,
    pub(super) additional: Vec<(D, Vec<ResourceId>)>,
}

impl<D> Commit<D> {
    /// Creates an event-bearing mutation; admission rejects an empty resource list.
    pub const fn new(op: OpId, data: D, affected: Vec<ResourceId>) -> Self {
        Self {
            op,
            data,
            affected,
            writes: Vec::new(),
            origin: None,
            additional: Vec::new(),
        }
    }

    /// Adds a state write to the same transaction as the event and receipt.
    #[must_use]
    pub fn write(mut self, write: CasWrite) -> Self {
        self.writes.push(write);
        self
    }

    /// Adds another ordered event to this same durable transaction.
    #[must_use]
    pub fn event(mut self, data: D, affected: Vec<ResourceId>) -> Self {
        self.additional.push((data, affected));
        self
    }

    /// Preserves the remote sequence separately from the local committed sequence.
    #[must_use]
    pub const fn mirrored_from(mut self, origin: RemoteOrigin) -> Self {
        self.origin = Some(origin);
        self
    }
}

/// Event body stored atomically with its sequence column and state writes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Body<D> {
    pub schema_version: u32,
    pub identity: RuntimeIdentity,
    pub affected: Vec<ResourceId>,
    pub origin: Option<RemoteOrigin>,
    pub data: D,
}

/// One committed event; delivery identity is added by the subscription layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Event<D> {
    /// Version of the durable typed envelope.
    pub schema_version: u32,
    /// Committed sequence and owning lifetime.
    #[serde(flatten)]
    pub cursor: Cursor,
    /// Core boot that committed this event.
    pub boot_id: ServerBootId,
    /// Name supplied by the typed data variant.
    #[serde(rename = "type")]
    pub event_type: String,
    /// All affected resources.
    pub affected: Vec<ResourceId>,
    /// Upstream ordering evidence for mirrors.
    pub origin: Option<RemoteOrigin>,
    /// Type-specific data.
    pub data: D,
}

impl<D: EventData> Body<D> {
    pub(super) fn into_event(self, seq: EventSeq) -> Event<D> {
        Event {
            schema_version: self.schema_version,
            cursor: Cursor {
                machine_id: self.identity.machine_id(),
                runtime_id: self.identity.runtime_id(),
                seq,
            },
            boot_id: self.identity.server_boot_id(),
            event_type: self.data.event_type().into(),
            affected: self.affected,
            origin: self.origin,
            data: self.data,
        }
    }
}

/// Independent filter evaluated after the snapshot boundary.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Filter {
    /// Empty matches all event types.
    pub types: Vec<String>,
    /// Empty matches all affected resources.
    pub resources: Vec<ResourceId>,
}

impl Filter {
    /// Selects event names and affected resource identities; empty lists match all.
    pub const fn new(types: Vec<String>, resources: Vec<ResourceId>) -> Self {
        Self { types, resources }
    }

    pub(super) fn matches<D>(&self, event: &Event<D>) -> bool {
        (self.types.is_empty() || self.types.contains(&event.event_type))
            && (self.resources.is_empty()
                || self.resources.iter().any(|id| event.affected.contains(id)))
    }
}

pub(super) fn subscription_id() -> SubscriptionId {
    SubscriptionId::new(uuid::Uuid::now_v7().to_string())
}

//! One atomic mutation: compare-and-swap writes, its events and its operation receipt.

use crate::ids::{EventKind, OpId, Payload, ResourceKey, Revision, Version};

/// Writes `value` to `key` only if the resource is at `expected`; `None` leaves a tombstone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CasWrite {
    pub(crate) key: ResourceKey,
    pub(crate) expected: Version,
    pub(crate) value: Option<Payload>,
}

impl CasWrite {
    /// A compare-and-swap of `key` from version `expected` to `value`.
    pub const fn new(key: ResourceKey, expected: Version, value: Option<Payload>) -> Self {
        Self {
            key,
            expected,
            value,
        }
    }
}

/// An event committed together with the mutation that caused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEvent {
    pub(crate) kind: EventKind,
    pub(crate) data: Payload,
}

impl NewEvent {
    /// An event of type `kind` carrying `data`.
    pub const fn new(kind: EventKind, data: Payload) -> Self {
        Self { kind, data }
    }
}

/// Resource writes, events and the operation result, committed all together or not at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mutation {
    pub(crate) op: OpId,
    pub(crate) result: Payload,
    pub(crate) writes: Vec<CasWrite>,
    pub(crate) events: Vec<NewEvent>,
}

impl Mutation {
    /// A mutation for operation `op` whose receipt stores `result`; every mutation emits an event.
    pub fn new(op: OpId, result: Payload, first_event: NewEvent) -> Self {
        Self {
            op,
            result,
            writes: Vec::new(),
            events: vec![first_event],
        }
    }

    /// Adds a compare-and-swap write.
    #[must_use]
    pub fn compare_and_swap(mut self, write: CasWrite) -> Self {
        self.writes.push(write);
        self
    }

    /// Adds another event; events take consecutive sequence numbers in insertion order.
    #[must_use]
    pub fn event(mut self, event: NewEvent) -> Self {
        self.events.push(event);
        self
    }

    pub(crate) fn largest_payload(&self) -> usize {
        let values = self.writes.iter().filter_map(|write| write.value.as_ref());
        let bodies = self.events.iter().map(|event| &event.data);
        values
            .chain(bodies)
            .map(Payload::len)
            .fold(self.result.len(), usize::max)
    }
}

/// What a committed mutation was assigned.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CommitReceipt {
    /// The store revision after the commit: the sequence number of its last event.
    pub revision: Revision,
    /// The sequence number of its first event.
    pub first_event: Revision,
    /// The new version of each written resource, in write order.
    pub versions: Vec<(ResourceKey, Version)>,
}

//! Per-subscription event selection decoded once from the protocol filter.

use std::collections::BTreeSet;

use ovis_terminal_protocol::schema::common::MachineRef;
use ovis_terminal_protocol::schema::methods::events::EventFilter;

use super::SubscriptionError;
use crate::events::{Event, ResourceId};
use crate::registry::{MachineId, TerminalId};

/// Which committed events one subscription delivers; absent dimensions match everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selector {
    types: Option<BTreeSet<String>>,
    terminals: Option<BTreeSet<TerminalId>>,
    machines: Option<BTreeSet<MachineId>>,
}

impl Selector {
    /// Selects every committed event.
    pub fn all() -> Self {
        Self::default()
    }

    /// Keeps only these event types.
    #[must_use]
    pub fn types(mut self, types: impl IntoIterator<Item = String>) -> Self {
        self.types = Some(types.into_iter().collect());
        self
    }

    /// Keeps only events that name one of these terminals among their affected resources.
    #[must_use]
    pub fn terminals(mut self, terminals: impl IntoIterator<Item = TerminalId>) -> Self {
        self.terminals = Some(terminals.into_iter().collect());
        self
    }

    /// Keeps only events committed by, or mirrored from, one of these machines.
    #[must_use]
    pub fn machines(mut self, machines: impl IntoIterator<Item = MachineId>) -> Self {
        self.machines = Some(machines.into_iter().collect());
        self
    }

    /// Decodes the protocol filter, rejecting selections this core cannot evaluate exactly.
    ///
    /// # Errors
    ///
    /// Returns `filter_invalid` for an empty list, a malformed identity, or a label,
    /// parent or machine-name selection that needs a registry lookup.
    pub fn decode(filter: &EventFilter) -> Result<Self, SubscriptionError> {
        if filter.labels.is_some() {
            return Err(SubscriptionError::UnresolvedSelector { field: "labels" });
        }
        if filter.children_of.is_some() {
            return Err(SubscriptionError::UnresolvedSelector {
                field: "children_of",
            });
        }
        let mut selector = Self::all();
        if let Some(types) = &filter.types {
            selector = selector.types(nonempty(types, "types")?.iter().cloned());
        }
        if let Some(terminals) = &filter.terminals {
            let parsed = nonempty(terminals, "terminals")?
                .iter()
                .map(|id| id.as_str().parse())
                .collect::<Result<Vec<TerminalId>, _>>()
                .map_err(|source| SubscriptionError::InvalidIdentity {
                    field: "terminals",
                    source,
                })?;
            selector = selector.terminals(parsed);
        }
        if let Some(machines) = &filter.machines {
            let parsed = nonempty(machines, "machines")?
                .iter()
                .map(machine)
                .collect::<Result<Vec<_>, _>>()?;
            selector = selector.machines(parsed);
        }
        Ok(selector)
    }

    /// Whether `event` belongs to this subscription.
    pub fn matches<D>(&self, event: &Event<D>) -> bool {
        let machine = event
            .origin
            .as_ref()
            .map_or(event.cursor.machine_id, |origin| origin.machine_id);
        self.types
            .as_ref()
            .is_none_or(|types| types.contains(&event.event_type))
            && self.terminals.as_ref().is_none_or(|terminals| {
                event.affected.iter().any(|resource| {
                    matches!(resource, ResourceId::Terminal(id) if terminals.contains(id))
                })
            })
            && self
                .machines
                .as_ref()
                .is_none_or(|machines| machines.contains(&machine))
    }
}

const fn nonempty<'a, T>(list: &'a [T], field: &'static str) -> Result<&'a [T], SubscriptionError> {
    if list.is_empty() {
        Err(SubscriptionError::EmptySelector { field })
    } else {
        Ok(list)
    }
}

fn machine(reference: &MachineRef) -> Result<MachineId, SubscriptionError> {
    match reference {
        MachineRef::MachineId(id) => {
            id.as_str()
                .parse()
                .map_err(|source| SubscriptionError::InvalidIdentity {
                    field: "machines",
                    source,
                })
        }
        MachineRef::Name(_) => Err(SubscriptionError::UnresolvedSelector {
            field: "machines.name",
        }),
        _ => Err(SubscriptionError::UnresolvedSelector { field: "machines" }),
    }
}

//! Explicit alternatives returned when a name does not select one resource.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::BoundaryError;
use crate::schema::ids::{MachineId, RuntimeLifetimeId, TerminalId};

/// One stable terminal identity, including the machine and lifetime that own it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TargetCandidate {
    /// Owning machine.
    pub machine_id: MachineId,
    /// Owning runtime lifetime.
    pub runtime_lifetime: RuntimeLifetimeId,
    /// Stable terminal identity.
    pub terminal_id: TerminalId,
}

impl TargetCandidate {
    /// Creates a fully qualified alternative; no human focus is involved.
    pub const fn new(
        machine_id: MachineId,
        runtime_lifetime: RuntimeLifetimeId,
        terminal_id: TerminalId,
    ) -> Self {
        Self {
            machine_id,
            runtime_lifetime,
            terminal_id,
        }
    }
}

impl BoundaryError {
    /// Carries the alternatives a caller must choose between without acting.
    #[must_use]
    pub fn with_candidates(mut self, candidates: Vec<TargetCandidate>) -> Self {
        if let Self::Domain { data } = &mut self {
            data.candidates = candidates;
        }
        self
    }
}

//! Telling a reconnect to the same runtime apart from a restart or a different runtime.

use ovis_terminal_protocol::schema::ids::{MachineId, RuntimeLifetimeId, ServerBootId};
use ovis_terminal_protocol::schema::methods::server::HelloResult;

/// The identity triple one hello result proves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointIdentity {
    machine: MachineId,
    runtime: RuntimeLifetimeId,
    boot: ServerBootId,
}

/// How a later hello relates to an earlier one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Continuity {
    /// The same control-server process answered.
    SameBoot,
    /// The control server restarted; holders and their programs kept their lifetime.
    ControlRestarted,
    /// A new runtime lifetime: earlier terminal state does not carry over.
    NewLifetime,
    /// Another machine answered at this address.
    OtherMachine,
}

impl EndpointIdentity {
    /// The identity a hello result reports.
    #[must_use]
    pub fn of(hello: &HelloResult) -> Self {
        Self {
            machine: hello.machine_id.clone(),
            runtime: hello.runtime_lifetime.clone(),
            boot: hello.server_boot_id.clone(),
        }
    }

    /// Classifies this identity against one observed `earlier`.
    #[must_use]
    pub fn continuity(&self, earlier: &Self) -> Continuity {
        if self.machine != earlier.machine {
            Continuity::OtherMachine
        } else if self.runtime != earlier.runtime {
            Continuity::NewLifetime
        } else if self.boot != earlier.boot {
            Continuity::ControlRestarted
        } else {
            Continuity::SameBoot
        }
    }
}

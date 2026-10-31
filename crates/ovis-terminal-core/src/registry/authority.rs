//! Registry identities as the identity family of authority scopes and resolved targets.

use ovis_terminal_authz::IdentityFamily;
use ovis_terminal_protocol::schema::ids::{DocName, PluginId};

use super::{MachineId, PaneId, RuntimeId, TabId, TerminalGeneration, TerminalId, WorkspaceId};

/// Authority scopes and targets built on the identities this registry issues; documents and
/// plugins have no registry family yet and use their protocol names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct RegistryIdentities;

impl IdentityFamily for RegistryIdentities {
    type Machine = MachineId;
    type Lifetime = RuntimeId;
    type Workspace = WorkspaceId;
    type Tab = TabId;
    type Pane = PaneId;
    type Terminal = TerminalId;
    type Incarnation = TerminalGeneration;
    type Document = DocName;
    type Plugin = PluginId;
}

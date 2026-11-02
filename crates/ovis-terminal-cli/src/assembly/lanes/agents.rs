//! Agent lanes: the production agent assembly (status, prompts recovered from the log,
//! reported conversation records and session binding, behind the prompt bridge gate),
//! provider history, the agent list and the agent tree.

use std::sync::Arc;

use ovis_terminal_agent::conversation::binding::Stores;
use ovis_terminal_agent::tree::TreeService;
use ovis_terminal_authz::validation::ScopedRoot;
use ovis_terminal_core::registry::RuntimeIdentity;
use ovis_terminal_server::dispatch::agent::AgentAssembly;
use ovis_terminal_server::dispatch::lanes::agent_status::{AgentStatusLane, GetTurn};
use ovis_terminal_server::dispatch::lanes::agent_tree::{AgentTreeLane, ListChildren};
use ovis_terminal_server::dispatch::lanes::boundary_policy::AgentHistory;

use super::{Dispatch, DispatchLog, Hosting, stage};
use crate::assembly::AssemblyError;
use crate::assembly::directory::Directory;

/// The agent domain one server lifetime serves: its status service binds program tokens
/// and its prompts survive planned restarts.
pub(in crate::assembly) type Agents = AgentAssembly<Arc<Directory>>;

/// Builds the lifetime's agent assembly over `directory`; no conversation store is
/// authorized and typed prompt answers are not delivered as keys.
pub(in crate::assembly) fn assemble(
    identity: RuntimeIdentity,
    directory: &Arc<Directory>,
) -> Result<Arc<Agents>, AssemblyError> {
    let stores = Stores::new(&[]).map_err(stage("authorize session stores"))?;
    Ok(Arc::new(AgentAssembly::new(
        identity,
        Arc::clone(directory),
        (stores, None),
    )))
}

/// `agent.history`: provider sessions below the state's authorized store root, listed as
/// inert data by an unrecorded read; a path leaving the root is `forbidden`.
fn history(dispatch: &mut Dispatch, hosting: &Hosting<'_>) -> Result<(), AssemblyError> {
    let root = hosting.state.join("agent-stores");
    std::fs::create_dir_all(&root).map_err(stage("create the agent store root"))?;
    let scoped = ScopedRoot::open(&root).map_err(stage("open the agent store root"))?;
    dispatch
        .register_read(AgentHistory::new(scoped, hosting.identity.machine_id()))
        .map_err(stage("register agent.history"))
}

/// Registers the agent assembly's methods with open prompts recovered from this boot's
/// log, the agent tree over the same services, and `agent.history`.
pub(super) fn register(
    dispatch: &mut Dispatch,
    hosting: &Hosting<'_>,
    log: &DispatchLog,
) -> Result<(), AssemblyError> {
    let agents = &hosting.agents;
    log.read_view(|snapshot| agents.prompts().recover(snapshot))
        .map_err(stage("recover open prompts"))?;
    agents
        .register(dispatch)
        .map_err(stage("register the agent assembly"))?;
    let turns = AgentStatusLane::new(Arc::clone(&hosting.directory), Arc::clone(agents.status()));
    dispatch
        .register(GetTurn::new(&turns))
        .map_err(stage("register turn.get"))?;
    dispatch
        .register_read(crate::assembly::session::SessionRead {
            actor: hosting.actor.clone(),
            status: Arc::clone(agents.status()),
            machine: hosting.identity.machine_id(),
        })
        .map_err(stage("register session.snapshot"))?;
    dispatch
        .register_read(crate::assembly::agent_list::AgentList {
            actor: hosting.actor.clone(),
            status: Arc::clone(agents.status()),
            machine: hosting.identity.machine_id(),
        })
        .map_err(stage("register agent.list"))?;
    let tree = Arc::new(TreeService::new(
        hosting.identity,
        (Arc::clone(agents.status()), Arc::clone(agents.prompts())),
    ));
    let children = AgentTreeLane::new(Arc::clone(&hosting.directory), tree);
    dispatch
        .register_read(ListChildren::new(&children))
        .map_err(stage("register agent.children"))?;
    history(dispatch, hosting)
}

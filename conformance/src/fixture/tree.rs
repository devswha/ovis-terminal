//! The agent tree over four fixture terminals whose programs start, exit and are ended
//! only through this module.
//!
//! `terminal.run` stands in for the launcher until todo 31 assembles it: it refuses a
//! finished or replaced parent before anything starts, starts the next program
//! incarnation of the named fixture terminal and links it in the same unit.
//! `fixture.exit` delivers a program's exit (or replays an old one) and carries out the
//! end orders it returns, one incarnation-checked kill at a time; `fixture.program`
//! reports what each terminal received.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ovis_terminal_agent::status::{ProcessOwner, StatusService};
use ovis_terminal_agent::tree::{Link, TreeError, TreeService};
use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::{Completion, OperationProducer};
use ovis_terminal_core::registry::{MachineId, RuntimeIdentity, TargetResolutionError, TerminalId};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::ids::ProcessToken;
use ovis_terminal_protocol::schema::methods::status::StatusTargetParams;
use ovis_terminal_protocol::schema::methods::terminal::{ChildPolicy, TerminalRunParams};
use ovis_terminal_protocol::schema::scalar::Incarnation;
use ovis_terminal_server::dispatch::lanes::agent_status::TerminalDirectory;
use ovis_terminal_server::dispatch::lanes::agent_tree::{AgentTreeLane, ListChildren};
use ovis_terminal_server::dispatch::{Access, Completed, DispatchError, Execution, Method};
use serde::Deserialize;
use serde_json::{Value, json};

use super::agents::{AgentParts, Directory};
use super::handler::{Dispatch, Event};
use super::prompts::Gate;
use super::serve::at;
use crate::error::RunnerError;

/// The launcher stand-in is served but never advertised: the real `terminal.run` is not
/// assembled until todo 31.
pub(crate) const LAUNCH_STAND_IN: &str = "terminal.run";

/// Fixture terminals the tree scenarios launch programs in.
pub(crate) const TREE_TERMINALS: [(u64, &str); 4] = [
    (6, "parent"),
    (7, "child-a"),
    (8, "child-b"),
    (9, "child-c"),
];

type Core = Coordinator<Event, OperationProducer>;

/// One fixture terminal's current program and what it received.
#[derive(Debug, Default, Clone, Copy)]
struct Program {
    incarnation: u64,
    live: bool,
    kills: u64,
}

/// The tree service, the fixture programs and what carries out exits and end orders.
#[derive(Debug)]
pub(crate) struct Tree {
    service: Arc<TreeService>,
    status: Arc<StatusService>,
    core: Arc<Core>,
    directory: Arc<Directory>,
    programs: Mutex<BTreeMap<TerminalId, Program>>,
    machine: MachineId,
}

fn domain(error: &TreeError) -> BoundaryError {
    BoundaryError::Domain {
        data: Box::new(error.error_data()),
    }
}

fn poisoned<T>(_: T) -> BoundaryError {
    domain(&TreeError::Poisoned)
}

impl Tree {
    fn resolve(&self, target: &Target) -> Result<TerminalId, BoundaryError> {
        self.directory
            .resolve(target)
            .map(|resolved| resolved.terminal)
            .map_err(|_| BoundaryError::invalid_params(Some("params.target")))
    }

    fn program(&self, terminal: TerminalId) -> Result<Program, BoundaryError> {
        let programs = self.programs.lock().map_err(poisoned)?;
        Ok(programs.get(&terminal).copied().unwrap_or_default())
    }

    /// Starts `owner` and hands it the scenario-known token `fixture-process-token-<name>-<n>`.
    fn start(&self, owner: ProcessOwner, name: &str) -> Result<(), TreeError> {
        let token = format!("fixture-process-token-{name}-{}", owner.process.get());
        self.status
            .tokens()
            .bind(&ProcessToken::new(token), owner)?;
        let mut programs = self.programs.lock().map_err(|_| TreeError::Poisoned)?;
        let program = programs.entry(owner.terminal).or_default();
        program.incarnation = owner.process.get();
        program.live = true;
        drop(programs);
        Ok(())
    }

    /// Ends `owner` if it is still the terminal's live program, counting a kill when
    /// `killed`; returns whether it was. A stale order or replayed exit ends nothing.
    fn end(&self, owner: ProcessOwner, killed: bool) -> Result<bool, BoundaryError> {
        let mut programs = self.programs.lock().map_err(poisoned)?;
        let ended = programs.get_mut(&owner.terminal).is_some_and(|program| {
            let current = program.live && program.incarnation == owner.process.get();
            if current {
                program.live = false;
                program.kills = program.kills.saturating_add(u64::from(killed));
            }
            current
        });
        drop(programs);
        Ok(ended)
    }

    /// Records `owner`'s exit, then kills and observes the exit of every child ordered
    /// to end with it, transitively.
    fn exit(&self, owner: ProcessOwner, was_live: bool) -> Result<Value, BoundaryError> {
        let internal = |error: TreeError| domain(&error);
        let status = |error| domain(&TreeError::from(error));
        if was_live {
            self.status.exited(&*self.core, owner).map_err(status)?;
        }
        let exit = self.service.exited(&*self.core, owner).map_err(internal)?;
        let ordered: Vec<String> = exit
            .end_orders
            .iter()
            .map(|o| o.terminal.to_string())
            .collect();
        let mut queue = exit.end_orders;
        let mut ended = Vec::new();
        while let Some(order) = queue.pop() {
            if !self.end(order, true)? {
                continue;
            }
            self.status.exited(&*self.core, order).map_err(status)?;
            let child = self.service.exited(&*self.core, order).map_err(internal)?;
            ended.push(order.terminal.to_string());
            queue.extend(child.end_orders);
        }
        let finished = if exit.replayed {
            "replayed"
        } else {
            "recorded"
        };
        Ok(json!({ "finished": finished, "end_orders": ordered, "ended": ended }))
    }
}

/// Registers `terminal.run` and `agent.children` over the tree fixture terminals.
pub(crate) fn register(
    dispatch: &mut Dispatch,
    ((status, directory), gate): (&AgentParts, &Gate),
    (runtime, core): (RuntimeIdentity, Arc<Core>),
) -> Result<Arc<Tree>, RunnerError> {
    let services = (Arc::clone(status), Arc::clone(&gate.prompts));
    let tree = Arc::new(TreeService::new(runtime, services));
    let lane = AgentTreeLane::new(Arc::clone(directory), Arc::clone(&tree));
    dispatch
        .register_read(ListChildren::new(&lane))
        .map_err(at("register agent.children"))?;
    let fixture = Arc::new(Tree {
        service: tree,
        status: Arc::clone(status),
        core,
        directory: Arc::clone(directory),
        programs: Mutex::new(BTreeMap::new()),
        machine: runtime.machine_id(),
    });
    dispatch
        .register(Run(Arc::clone(&fixture)))
        .map_err(at("register terminal.run"))?;
    Ok(fixture)
}

/// `terminal.run` over the tree fixture terminals.
#[derive(Debug)]
struct Run(Arc<Tree>);

impl Method<Event, OperationProducer> for Run {
    const NAME: &'static str = LAUNCH_STAND_IN;
    type Params = TerminalRunParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        let machine = ResolvedTarget::new(self.0.machine, TargetResource::Machine);
        Ok(Access::new(Action::Launch, machine))
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<Event, OperationProducer>,
    ) -> Result<Completed, DispatchError> {
        let unnamed = TargetResolutionError::NeedsTarget;
        let name = params.name.clone().ok_or(unnamed)?;
        let target = Target::Name(name.clone());
        let terminal = self.0.directory.resolve(&target)?.terminal;
        let fixture = Arc::clone(&self.0);
        execution
            .commit_unit(move |snapshot| {
                TreeError::settle((|| {
                    if let Some(parent) = &params.parent {
                        fixture.service.admit(snapshot, parent)?;
                    }
                    let next = fixture
                        .program(terminal)
                        .map_or(1, |p| p.incarnation.saturating_add(1));
                    let owner = ProcessOwner::new(terminal, Incarnation::new(next));
                    let mut completion = Completion::new(json!({
                        "terminal_id": terminal.to_string(),
                        "process_incarnation": next.to_string(),
                    }));
                    if let Some(parent) = params.parent {
                        let policy = params.child_policy.unwrap_or(ChildPolicy::new(true, true));
                        let linked = fixture
                            .service
                            .link(snapshot, Link::new(parent, owner, policy))?;
                        for write in linked.writes {
                            completion = completion.write(write);
                        }
                    }
                    fixture.start(owner, name.as_str())?;
                    Ok(completion)
                })())
            })
            .await
    }
}

/// Parameters of `fixture.exit`: the terminal, and the incarnation to replay if not its current one.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExitParams {
    target: Target,
    incarnation: Option<Incarnation>,
}

/// Delivers a program's exit, or replays an older incarnation's.
pub(crate) async fn exit(tree: &Arc<Tree>, request: &RpcRequest) -> Result<Value, BoundaryError> {
    let params: ExitParams = decode_params(request).map_err(BoundaryError::from)?;
    let terminal = tree.resolve(&params.target)?;
    let current = tree.program(terminal)?.incarnation;
    let incarnation = params
        .incarnation
        .unwrap_or_else(|| Incarnation::new(current));
    let owner = ProcessOwner::new(terminal, incarnation);
    let tree = Arc::clone(tree);
    tokio::task::spawn_blocking(move || {
        let live = tree.end(owner, false)?;
        tree.exit(owner, live)
    })
    .await
    .map_err(poisoned)?
}

/// What the named fixture terminal's programs received.
pub(crate) fn program(tree: &Tree, request: &RpcRequest) -> Result<Value, BoundaryError> {
    let params: StatusTargetParams = decode_params(request).map_err(BoundaryError::from)?;
    let program = tree.program(tree.resolve(&params.target)?)?;
    Ok(json!({
        "incarnation": program.incarnation.to_string(),
        "live": program.live,
        "kills": program.kills.to_string(),
    }))
}

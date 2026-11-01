//! The fixture's registration phases: each lane family registers on the one dispatcher and
//! hands back what the connection handler and the stop sequence need.

use std::path::Path;
use std::sync::Arc;

use ovis_terminal_agent::bridge::PromptBridge;
use ovis_terminal_agent::prompts::PromptService;
use ovis_terminal_agent::status::StatusService;
use ovis_terminal_authz::Authority;
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity};
use ovis_terminal_server::dispatch::lanes::boundary_policy::AuditTrail;

use super::super::bridge::{Extension, QUEUE_LIMIT};
use super::super::documents::{DocGet, DocSet};
use super::super::handler::{Dispatch, Event, Probes};
use super::super::terminals::TerminalKeep;
use super::super::update::Held;
use super::at;
use crate::error::RunnerError;

/// The fixture's coordinator.
type Core = Coordinator<Event, OperationProducer>;
/// The authority, the owner bearer and the coordinator every phase shares.
pub(super) type Shared<'a> = (
    &'a Arc<Authority<RegistryIdentities>>,
    &'a str,
    &'a Arc<Core>,
);

/// What the registration phases produced.
pub(super) struct Registered {
    /// The stand-in probes the handler answers.
    pub(super) probes: Probes,
    /// Agent status, shared with the prompt bridge gate.
    pub(super) status: Arc<StatusService>,
    /// Open prompts, shared with the prompt bridge gate.
    pub(super) prompts: Arc<PromptService>,
    /// The update lane's holder, shut down after the dispatcher.
    pub(super) held: Held,
    /// The audit trail the boundary policy records every call in.
    pub(super) trail: Arc<AuditTrail>,
}

/// `doc.set`, `doc.get` and `terminal.keep` over the effect directory below `root`.
fn documents(
    dispatch: &mut Dispatch,
    root: &Path,
    runtime: RuntimeIdentity,
) -> Result<(), RunnerError> {
    let effects = root.join("effects");
    std::fs::create_dir(&effects).map_err(at("create the effect directory"))?;
    dispatch
        .register(DocSet::new(&effects, runtime))
        .map_err(at("register doc.set"))?;
    dispatch
        .register(DocGet::new(&effects, runtime))
        .map_err(at("register doc.get"))?;
    dispatch
        .register(TerminalKeep::new(&effects, runtime)?)
        .map_err(at("register terminal.keep"))
}

/// Registers every fixture lane on `dispatch`, one phase per lane family.
pub(super) async fn register(
    dispatch: &mut Dispatch,
    (root, runtime): (&Path, RuntimeIdentity),
    (authority, bearer, core): Shared<'_>,
) -> Result<Registered, RunnerError> {
    documents(dispatch, root, runtime)?;
    let (agents, gate, sequenced) =
        super::super::agents::register(dispatch, (runtime, Arc::clone(core)), authority)?;
    super::super::rotation::register(dispatch, authority, (runtime, Arc::clone(&sequenced)))?;
    let (status, directory) = (Arc::clone(&agents.0), Arc::clone(&agents.1));
    super::super::prompts::register(dispatch, (&gate, Arc::clone(&directory)), runtime)?;
    let programs = super::super::processes::register(
        dispatch,
        (root, runtime),
        (authority, bearer, &agents.0),
    )
    .await?;
    let parts = (authority, &**core, agents.clone());
    let template = super::super::template_lane::register(dispatch, (runtime, root), parts)?;
    let launch = (runtime, Arc::clone(core), template);
    let tree = super::super::tree::register(dispatch, (&agents, &gate), launch)?;
    let fleet = (runtime, Arc::clone(core));
    let machines = super::super::machines::register(dispatch, (&agents, &gate), fleet).await?;
    super::super::transcripts::register(dispatch, agents, (runtime, root))?;
    let health = super::super::health::register(dispatch, runtime, root)?;
    let held = super::super::update::register(dispatch, (root, authority, bearer), runtime).await?;
    let trail = super::super::boundary::register(dispatch, root, runtime)?;
    let prompts = Arc::clone(&gate.prompts);
    let bridge = PromptBridge::new(Arc::clone(&status), Arc::clone(&prompts), QUEUE_LIMIT);
    let probes = Probes {
        gate,
        directory,
        extension: Extension::new(bridge, Arc::clone(core)),
        sequenced,
        tree,
        health,
        machines,
        programs,
        audit: (Arc::clone(&trail), Arc::clone(authority)),
    };
    Ok(Registered {
        probes,
        status,
        prompts,
        held,
        trail,
    })
}

//! Behavior of parent/child links over a real store, coordinator and operation ledger.

mod exits;
mod links;
mod remote;

use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_core::events::{Coordinator, EventError};
use ovis_terminal_core::ops::{
    Completion, Context, OperationError, OperationProducer, Operations, Request,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId, TerminalId,
};
use ovis_terminal_protocol::domain::agent_tree::ChildLink;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::{ProcessToken, TerminalId as WireTerminal};
use ovis_terminal_protocol::schema::methods::agent::AgentChildrenResult;
use ovis_terminal_protocol::schema::methods::server::OpRecord;
use ovis_terminal_protocol::schema::methods::terminal::ChildPolicy;
use ovis_terminal_protocol::schema::records::ParentRef;
use ovis_terminal_protocol::schema::scalar::{Incarnation, TimestampMs};
use ovis_terminal_store::{Snapshot, Store, StoreOptions};

use super::{Exit, Link, TreeError, TreeService};
use crate::prompts::PromptService;
use crate::status::{AgentStatusEvent, ProcessOwner, StatusService};

type TestResult = Result<(), Box<dyn Error>>;
type Core = Coordinator<AgentStatusEvent, OperationProducer>;

const PARENT: u64 = 6;
const OPTED_IN: u64 = 7;
const INDEPENDENT: u64 = 8;

const DIE: ChildPolicy = ChildPolicy::new(true, true);
const KEEP: ChildPolicy = ChildPolicy::new(false, false);

fn terminal(id: u64) -> Result<TerminalId, Box<dyn Error>> {
    Ok(TerminalId::try_from(id)?)
}

fn owner(id: u64, incarnation: u64) -> Result<ProcessOwner, Box<dyn Error>> {
    Ok(ProcessOwner::new(
        terminal(id)?,
        Incarnation::new(incarnation),
    ))
}

fn token(id: u64, incarnation: u64) -> ProcessToken {
    ProcessToken::new(format!("token-{id}-{incarnation}"))
}

fn parent_ref(id: u64, incarnation: u64) -> Result<ParentRef, Box<dyn Error>> {
    Ok(ParentRef::new(
        WireTerminal::new(terminal(id)?.to_string()),
        Incarnation::new(incarnation),
    ))
}

struct Harness {
    root: PathBuf,
    core: Core,
    status: Arc<StatusService>,
    prompts: Arc<PromptService>,
    tree: TreeService,
    identity: RuntimeIdentity,
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.root) {
            eprintln!("cannot remove {}: {error}", self.root.display());
        }
    }
}

fn harness() -> Result<Harness, Box<dyn Error>> {
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000041".parse()?;
    let identity = RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(41)?,
        ServerBootId::try_from(1)?,
    );
    let root = std::env::temp_dir().join(format!("ovis-t41-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&root)?;
    let store = Store::open(&root.join("state"), StoreOptions::default())?;
    let producer = OperationProducer::new(identity.runtime_id());
    let core = Coordinator::new(store, identity, producer);
    let status = Arc::new(StatusService::new(identity.runtime_id()));
    let prompts = Arc::new(PromptService::new(
        identity.runtime_id(),
        Arc::clone(&status),
    ));
    let tree = TreeService::new(identity, (Arc::clone(&status), Arc::clone(&prompts)));
    Ok(Harness {
        root,
        core,
        status,
        prompts,
        tree,
        identity,
    })
}

impl Harness {
    /// Starts program `incarnation` in terminal `id`, retiring the previous one's token.
    fn launch(&self, id: u64, incarnation: u64) -> Result<ProcessOwner, Box<dyn Error>> {
        let launched = owner(id, incarnation)?;
        self.status
            .tokens()
            .bind(&token(id, incarnation), launched)?;
        Ok(launched)
    }

    /// Runs one operation through accept, start and an atomic commit unit.
    fn op(
        &self,
        build: impl FnOnce(&Snapshot) -> Result<Completion<AgentStatusEvent>, OperationError>,
    ) -> Result<OpRecord, Box<dyn Error>> {
        let op = OpId::new(uuid::Uuid::now_v7().to_string());
        let request = Request::new(op, "tree.test".to_owned(), serde_json::json!({}))?;
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis();
        let now = TimestampMs::new(u64::try_from(millis)?);
        let context = Context::new(PrincipalId::parse("owner")?, self.identity, now);
        let operations = Operations::new(&self.core, context);
        operations.accept(&request)?;
        let permit = operations.start(request.op_id())?;
        Ok(operations.commit_unit(permit, build)?)
    }

    fn run(
        &self,
        build: impl FnOnce(&Snapshot) -> Result<Completion<AgentStatusEvent>, TreeError>,
    ) -> Result<OpRecord, Box<dyn Error>> {
        self.op(|snapshot| TreeError::settle(build(snapshot)))
    }

    /// Launches `child` and links it to `parent` in one unit, as `terminal.run` does.
    fn link(
        &self,
        parent: ParentRef,
        (child, policy): (ProcessOwner, ChildPolicy),
    ) -> Result<Result<ChildLink, String>, Box<dyn Error>> {
        let record = self.run(|snapshot| {
            let linked = self.tree.link(snapshot, Link::new(parent, child, policy))?;
            let mut completion = Completion::new(serde_json::to_value(&linked.link)?);
            for write in linked.writes {
                completion = completion.write(write);
            }
            Ok(completion)
        })?;
        Ok(match (record.result, record.error) {
            (Some(result), None) => Ok(serde_json::from_value(result)?),
            (_, error) => Err(error.map_or_else(String::new, |e| e.kind.token().to_owned())),
        })
    }

    fn exited(&self, program: ProcessOwner) -> Result<Exit, TreeError> {
        self.tree.exited(&self.core, program)
    }

    fn children(&self, id: u64) -> Result<AgentChildrenResult, Box<dyn Error>> {
        let terminal = terminal(id)?;
        Ok(self.read(|snapshot| self.tree.children(snapshot, terminal))??)
    }

    fn read<T>(&self, read: impl FnOnce(&Snapshot) -> T) -> Result<T, Box<dyn Error>> {
        Ok(self
            .core
            .read_view(|snapshot| Ok::<_, EventError>(read(snapshot)))?)
    }
}

//! The lane over a real store, coordinator, agent tree and machine lines whose remote ends
//! are driven by each test.

mod orphans;
mod pump;

use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_agent::prompts::PromptService;
use ovis_terminal_agent::status::{AgentStatusEvent, ProcessOwner, StatusService};
use ovis_terminal_agent::tree::{Exit, TreeError, TreeService};
use ovis_terminal_authz::PrincipalId;
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::{Completion, Context, OperationProducer, Operations, Request};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, RuntimeId, RuntimeIdentity, ServerBootId, TerminalId,
};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::{MachineId, ProcessToken, TerminalId as WireTerminal};
use ovis_terminal_protocol::schema::methods::machine::MachineRecord;
use ovis_terminal_protocol::schema::records::ParentRef;
use ovis_terminal_protocol::schema::scalar::{Incarnation, TimestampMs};
use ovis_terminal_remote::actors::{Actors, Inbound, Limits, Lines};
use ovis_terminal_store::{Snapshot, Store, StoreOptions};
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream, ReadHalf, WriteHalf,
};
use tokio::sync::mpsc;

use super::{MachineActorLane, TreeOutbox};

type TestResult = Result<(), Box<dyn Error>>;
type Core = Coordinator<AgentStatusEvent, OperationProducer>;
type PipeLine = Lines<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>;
type Lane = MachineActorLane<PipeLine, AgentStatusEvent, OperationProducer>;

const HANG: Duration = Duration::from_secs(10);
const LOCAL: &str = "machine:0191f1a2-0000-7000-8000-000000000044";

fn bravo() -> MachineId {
    MachineId::new("machine:0191f1a2-0000-7000-8000-0000000000b2".to_owned())
}

fn parent_ref(id: u64, incarnation: u64) -> Result<ParentRef, Box<dyn Error>> {
    let terminal = TerminalId::try_from(id)?;
    Ok(ParentRef::new(
        WireTerminal::new(terminal.to_string()),
        Incarnation::new(incarnation),
    ))
}

/// The remote end of one machine line.
struct Peer {
    reader: BufReader<ReadHalf<DuplexStream>>,
    writer: WriteHalf<DuplexStream>,
}

impl Peer {
    async fn next(&mut self) -> Result<Value, Box<dyn Error>> {
        let mut line = String::new();
        if tokio::time::timeout(HANG, self.reader.read_line(&mut line)).await?? == 0 {
            return Err("the line closed".into());
        }
        Ok(serde_json::from_str(&line)?)
    }

    async fn write(&mut self, record: &Value) -> TestResult {
        self.writer
            .write_all(format!("{record}\n").as_bytes())
            .await?;
        Ok(())
    }
}

/// The value at `pointer` in `value`, or null.
fn at(value: &Value, pointer: &str) -> Value {
    value.pointer(pointer).cloned().unwrap_or(Value::Null)
}

fn pipe() -> (PipeLine, Peer) {
    let (local, remote) = tokio::io::duplex(64 * 1024);
    let (local_read, local_write) = tokio::io::split(local);
    let (remote_read, remote_write) = tokio::io::split(remote);
    let peer = Peer {
        reader: BufReader::new(remote_read),
        writer: remote_write,
    };
    (Lines::new(local_read, local_write), peer)
}

struct Harness {
    root: PathBuf,
    core: Arc<Core>,
    status: Arc<StatusService>,
    tree: Arc<TreeService>,
    identity: RuntimeIdentity,
}

impl Drop for Harness {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

fn harness() -> Result<Harness, Box<dyn Error>> {
    let identity = RuntimeIdentity::new(
        HostIdentity::new(LOCAL.parse()?, HostBootId::try_from(1)?),
        RuntimeId::try_from(44)?,
        ServerBootId::try_from(1)?,
    );
    let root = std::env::temp_dir().join(format!("ovis-t44-lane-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&root)?;
    let store = Store::open(&root.join("state"), StoreOptions::default())?;
    let core = Arc::new(Coordinator::new(
        store,
        identity,
        OperationProducer::new(identity.runtime_id()),
    ));
    let status = Arc::new(StatusService::new(identity.runtime_id()));
    let prompts = Arc::new(PromptService::new(
        identity.runtime_id(),
        Arc::clone(&status),
    ));
    let tree = Arc::new(TreeService::new(identity, (Arc::clone(&status), prompts)));
    Ok(Harness {
        root,
        core,
        status,
        tree,
        identity,
    })
}

impl Harness {
    /// Starts program `incarnation` of terminal `id`.
    fn launch(&self, id: u64, incarnation: u64) -> Result<ProcessOwner, Box<dyn Error>> {
        let owner = ProcessOwner::new(TerminalId::try_from(id)?, Incarnation::new(incarnation));
        let token = ProcessToken::new(format!("token-{id}-{incarnation}"));
        self.status.tokens().bind(&token, owner)?;
        Ok(owner)
    }

    /// Commits `build`'s writes as one operation, as a launch does.
    fn run(
        &self,
        build: impl FnOnce(&Snapshot) -> Result<Vec<ovis_terminal_store::CasWrite>, TreeError>,
    ) -> TestResult {
        let request = Request::new(
            OpId::new(uuid::Uuid::now_v7().to_string()),
            "tree.test".to_owned(),
            json!({}),
        )?;
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis();
        let now = TimestampMs::new(u64::try_from(millis)?);
        let context = Context::new(PrincipalId::parse("owner")?, self.identity, now);
        let operations = Operations::new(&*self.core, context);
        operations.accept(&request)?;
        let permit = operations.start(request.op_id())?;
        operations.commit_unit(permit, |snapshot| {
            TreeError::settle(build(snapshot).map(|writes| {
                writes
                    .into_iter()
                    .fold(Completion::new(json!({})), Completion::write)
            }))
        })?;
        Ok(())
    }

    fn exited(&self, owner: ProcessOwner) -> Result<Exit, TreeError> {
        self.tree.exited(&*self.core, owner)
    }

    fn lane(
        &self,
        deadline: Duration,
    ) -> Result<(Arc<Lane>, mpsc::Receiver<Inbound>), Box<dyn Error>> {
        let (actors, inbound) = Actors::new(Limits::new(8, Duration::from_secs(60), 64));
        let record: MachineRecord = serde_json::from_value(json!({
            "machine_id": bravo(), "name": "bravo", "aliases": [], "state": "connected",
            "remote_lifetime": "runtime:7", "remote_boot_id": "1", "last_seen_seq": null,
            "version": "1.0.0", "revision": "1",
        }))?;
        actors.add(record)?;
        let outbox = TreeOutbox::new(Arc::clone(&self.tree), Arc::clone(&self.core));
        let lane = MachineActorLane::new(
            self.identity.machine_id(),
            (Arc::new(actors), outbox),
            deadline,
        );
        Ok((lane, inbound))
    }
}

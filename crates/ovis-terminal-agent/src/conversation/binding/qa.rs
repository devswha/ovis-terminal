//! Behavior of session binding and history cursors over a real store, coordinator,
//! operation ledger and transcript files.

mod binding;
mod chain;
mod cursor;
mod observe;

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_core::events::{Coordinator, Event, Subscription};
use ovis_terminal_core::ops::{
    Completion, Context, OperationError, OperationProducer, OperationSnapshot, Operations, Request,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId, TerminalId,
};
use ovis_terminal_protocol::domain::session_binding::{ConversationCursor, SessionChange};
use ovis_terminal_protocol::errors::{ErrorData, OpId};
use ovis_terminal_protocol::schema::ids::{HistoryId, ProcessToken, RecordId};
use ovis_terminal_protocol::schema::methods::agent::{AgentConversationResult, AgentResult};
use ovis_terminal_protocol::schema::methods::server::OpRecord;
use ovis_terminal_protocol::schema::records::SessionRef;
use ovis_terminal_protocol::schema::scalar::{Incarnation, TimestampMs};
use ovis_terminal_store::{Snapshot, Store, StoreOptions};
use serde::de::DeserializeOwned;
use serde_json::json;

use super::{BindingService, Read, Stores, Update};
use crate::status::{AgentStatusEvent, ProcessOwner, Register, StatusService};

type TestResult = Result<(), Box<dyn Error>>;
type Core = Coordinator<AgentStatusEvent, OperationProducer>;

const AGENT: u64 = 4;
const TOKEN: &str = "token-agent-4";

fn terminal() -> Result<TerminalId, Box<dyn Error>> {
    Ok(TerminalId::try_from(AGENT)?)
}

fn token(text: &str) -> ProcessToken {
    ProcessToken::new(text.to_owned())
}

fn now() -> Result<TimestampMs, Box<dyn Error>> {
    Ok(TimestampMs::new(u64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?))
}

fn history(id: &str) -> HistoryId {
    HistoryId::new(id.to_owned())
}

fn record(id: &str) -> RecordId {
    RecordId::new(id.to_owned())
}

/// A transcript line; `parent: None` continues the previous line.
fn line(id: &str, parent: Option<&str>) -> String {
    let mut value = json!({"id": id, "role": "user", "blocks": [{"text": id}]});
    if let (Some(parent), Some(object)) = (parent, value.as_object_mut()) {
        object.insert("parent".to_owned(), json!(parent));
    }
    format!("{value}\n")
}

struct Harness {
    root: PathBuf,
    core: Core,
    status: Arc<StatusService>,
    service: BindingService,
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
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000036".parse()?;
    let identity = RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(36)?,
        ServerBootId::try_from(1)?,
    );
    let root = std::env::temp_dir().join(format!("ovis-t36-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(root.join("store"))?;
    let store = Store::open(&root.join("state"), StoreOptions::default())?;
    let core = Coordinator::new(
        store,
        identity,
        OperationProducer::new(identity.runtime_id()),
    );
    let status = Arc::new(StatusService::new(identity.runtime_id()));
    status.tokens().bind(
        &token(TOKEN),
        ProcessOwner::new(terminal()?, Incarnation::new(1)),
    )?;
    let stores = Stores::new(&[("fixture-store", root.join("store"))])?;
    let service = BindingService::new(identity.runtime_id(), Arc::clone(&status), stores);
    Ok(Harness {
        root,
        core,
        status,
        service,
        identity,
    })
}

fn result<T: DeserializeOwned>(record: &OpRecord) -> Result<T, Box<dyn Error>> {
    Ok(serde_json::from_value(
        record
            .result
            .clone()
            .ok_or_else(|| format!("no result: {record:?}"))?,
    )?)
}

fn refusal(record: &OpRecord) -> Result<&ErrorData, Box<dyn Error>> {
    record
        .error
        .as_ref()
        .ok_or_else(|| format!("expected a refusal, got {record:?}").into())
}

/// Domain events delivered to `subscription`, skipping operation receipts.
fn domain_events(
    subscription: &Subscription<AgentStatusEvent, OperationSnapshot>,
) -> Vec<Event<AgentStatusEvent>> {
    let mut events = Vec::new();
    while let Ok(event) = subscription
        .delivery
        .recv_timeout(Duration::from_millis(50))
    {
        if !matches!(event.data, AgentStatusEvent::Operation(_)) {
            events.push(event);
        }
    }
    events
}

impl Harness {
    fn run(
        &self,
        build: impl FnOnce(&Snapshot) -> Result<Completion<AgentStatusEvent>, OperationError>,
    ) -> Result<OpRecord, Box<dyn Error>> {
        let id = OpId::new(uuid::Uuid::now_v7().to_string());
        let request = Request::new(id, "binding.test".to_owned(), json!({}))?;
        let operations = Operations::new(
            &self.core,
            Context::new(PrincipalId::parse("owner")?, self.identity, now()?),
        );
        operations.accept(&request)?;
        let permit = operations.start(request.op_id())?;
        Ok(operations.commit_unit(permit, build)?)
    }

    fn path(&self, file: &str) -> PathBuf {
        self.root.join("store").join(file)
    }

    fn write(&self, file: &str, lines: &[String]) -> Result<PathBuf, Box<dyn Error>> {
        let path = self.path(file);
        std::fs::write(&path, lines.concat())?;
        Ok(path)
    }

    fn register(&self) -> Result<AgentResult, Box<dyn Error>> {
        let register = Register::new(terminal()?, token(TOKEN), "custom-agent".to_owned());
        result(&self.run(|snapshot| self.status.register(snapshot, register))?)
    }

    fn agent(&self) -> Result<AgentResult, Box<dyn Error>> {
        let terminal = terminal()?;
        result(&self.run(|snapshot| self.status.agent(snapshot, terminal))?)
    }

    fn update(&self, update: Update) -> Result<OpRecord, Box<dyn Error>> {
        self.run(|snapshot| self.service.update_session(snapshot, update))
    }

    /// Binds `path` as `id` with `reason`, presenting `TOKEN`.
    fn bind(
        &self,
        (path, id): (&Path, &str),
        reason: Option<SessionChange>,
    ) -> Result<OpRecord, Box<dyn Error>> {
        let mut update = Update::new(terminal()?, token(TOKEN), session(path)?);
        update.history_id = Some(history(id));
        update.reason = reason;
        self.update(update)
    }

    fn read(&self, cursor: Option<ConversationCursor>) -> Result<OpRecord, Box<dyn Error>> {
        let mut read = Read::new(terminal()?);
        read.cursor = cursor;
        self.run(|snapshot| self.service.conversation(snapshot, &read))
    }

    fn page(
        &self,
        cursor: Option<ConversationCursor>,
    ) -> Result<AgentConversationResult, Box<dyn Error>> {
        result(&self.read(cursor)?)
    }
}

fn session(path: &Path) -> Result<SessionRef, Box<dyn Error>> {
    Ok(serde_json::from_value(json!({
        "source": "fixture-store", "kind": "path", "value": path.to_string_lossy()
    }))?)
}

/// Record ids of a page, in order.
fn ids(page: &AgentConversationResult) -> Vec<&str> {
    page.records
        .iter()
        .map(|record| record.record_id.as_str())
        .collect()
}

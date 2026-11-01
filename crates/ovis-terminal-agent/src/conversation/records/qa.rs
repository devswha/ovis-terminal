//! Behavior of typed record appends, reads and turn records over a real store,
//! coordinator, operation ledger and agent status service.

mod append;
mod bounds;
mod refusals;
mod split;
mod turns;

use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_core::events::{Coordinator, EventError, Subscription};
use ovis_terminal_core::ops::{
    Completion, Context, OperationError, OperationProducer, OperationSnapshot, Operations, Request,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId, TerminalId,
};
use ovis_terminal_protocol::domain::conversation_records::{
    RecordInput, RecordsAppended, TurnMetadata,
};
use ovis_terminal_protocol::domain::session_binding::ConversationCursor;
use ovis_terminal_protocol::errors::{ErrorData, OpId};
use ovis_terminal_protocol::schema::ids::ProcessToken;
use ovis_terminal_protocol::schema::methods::agent::AgentConversationResult;
use ovis_terminal_protocol::schema::methods::server::OpRecord;
use ovis_terminal_protocol::schema::records::{AgentState, StatusReport};
use ovis_terminal_protocol::schema::scalar::{Incarnation, ReportSeq, TimestampMs};
use ovis_terminal_store::{Snapshot, Store, StoreOptions};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{Append, RecordsError, RecordsService};
use crate::conversation::binding::Read;
use crate::status::{AgentStatusEvent, ProcessOwner, Register, Report, StatusService};

type TestResult = Result<(), Box<dyn Error>>;
type Core = Coordinator<AgentStatusEvent, OperationProducer>;
type Events = Subscription<AgentStatusEvent, OperationSnapshot>;

const AGENT: u64 = 4;
const SHELL: u64 = 5;
const TOKEN: &str = "token-agent-4";

fn terminal(id: u64) -> Result<TerminalId, Box<dyn Error>> {
    Ok(TerminalId::try_from(id)?)
}

fn token(text: &str) -> ProcessToken {
    ProcessToken::new(text.to_owned())
}

/// Records decoded from their wire form, as the dispatcher would hand them over.
fn inputs(records: &Value) -> Result<Vec<RecordInput>, Box<dyn Error>> {
    Ok(serde_json::from_value(records.clone())?)
}

fn text(record: &str, role: &str, text: &str) -> Value {
    json!({"record_id": record, "role": role, "blocks": [{"type": "text", "text": text}]})
}

/// The happy conversation: a question, commentary with a call, the user's mixed text
/// and result, then the final answer.
fn conversation() -> Value {
    json!([
        text("u1", "user", "fix the build"),
        {"record_id": "a1", "role": "assistant", "phase": "commentary", "blocks": [
            {"type": "text", "text": "reading the manifest"},
            {"type": "tool_call", "call_id": "c1", "name": "read", "input": {"path": "Cargo.toml"}}
        ]},
        {"record_id": "u2", "role": "user", "at_ms": "1700000000123", "blocks": [
            {"type": "text", "text": "here it is"},
            {"type": "tool_result", "call_id": "c1", "output": "[package]", "is_error": false}
        ]},
        {"record_id": "a2", "role": "assistant", "phase": "final", "blocks": [
            {"type": "text", "text": "fixed"}
        ]}
    ])
}

struct Harness {
    root: std::path::PathBuf,
    core: Core,
    status: Arc<StatusService>,
    records: RecordsService,
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
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000037".parse()?;
    let identity = RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(37)?,
        ServerBootId::try_from(1)?,
    );
    let root = std::env::temp_dir().join(format!("ovis-t37-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&root)?;
    let store = Store::open(&root.join("state"), StoreOptions::default())?;
    let core = Coordinator::new(
        store,
        identity,
        OperationProducer::new(identity.runtime_id()),
    );
    let status = Arc::new(StatusService::new(identity.runtime_id()));
    for (id, name) in [(AGENT, TOKEN), (SHELL, "token-shell-5")] {
        let owner = ProcessOwner::new(terminal(id)?, Incarnation::new(1));
        status.tokens().bind(&token(name), owner)?;
    }
    let records = RecordsService::new(identity.runtime_id(), Arc::clone(&status));
    Ok(Harness {
        root,
        core,
        status,
        records,
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

/// The next `agent.records_appended` event; operation receipts and status events pass.
fn next_appended(events: &Events) -> Result<RecordsAppended, Box<dyn Error>> {
    loop {
        let event = events.delivery.recv_timeout(Duration::from_secs(10))?;
        if let AgentStatusEvent::RecordsAppended(appended) = event.data {
            return Ok(*appended);
        }
    }
}

impl Harness {
    fn run(
        &self,
        build: impl FnOnce(&Snapshot) -> Result<Completion<AgentStatusEvent>, OperationError>,
    ) -> Result<OpRecord, Box<dyn Error>> {
        let id = OpId::new(uuid::Uuid::now_v7().to_string());
        let request = Request::new(id, "records.test".to_owned(), json!({}))?;
        let started = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let operations = Operations::new(
            &self.core,
            Context::new(
                PrincipalId::parse("owner")?,
                self.identity,
                TimestampMs::new(u64::try_from(started)?),
            ),
        );
        operations.accept(&request)?;
        let permit = operations.start(request.op_id())?;
        Ok(operations.commit_unit(permit, build)?)
    }

    fn register(&self) -> Result<OpRecord, Box<dyn Error>> {
        let register = Register::new(terminal(AGENT)?, token(TOKEN), "custom-agent".to_owned());
        self.run(|snapshot| self.status.register(snapshot, register))
    }

    /// The owner reports `state` as producer sequence `seq`, accepted at `at` ms.
    fn report(&self, state: AgentState, (seq, at): (u64, u64)) -> Result<OpRecord, Box<dyn Error>> {
        let mut report = Report::new(terminal(AGENT)?, token(TOKEN), StatusReport::new(state));
        report.seq = ReportSeq::new(seq);
        report.at = TimestampMs::new(at);
        self.run(|snapshot| self.status.report(snapshot, report))
    }

    fn append_as(
        &self,
        (by, records): (&str, &Value),
        turn: Option<TurnMetadata>,
    ) -> Result<OpRecord, Box<dyn Error>> {
        let mut append = Append::new(terminal(AGENT)?, token(by), inputs(records)?);
        append.turn = turn;
        self.run(|snapshot| self.records.append(snapshot, append))
    }

    fn append(&self, records: &Value) -> Result<OpRecord, Box<dyn Error>> {
        self.append_as((TOKEN, records), None)
    }

    /// Reads like `agent.records`: from the latest committed snapshot, no receipt.
    fn read(
        &self,
        read: &Read,
    ) -> Result<Result<AgentConversationResult, RecordsError>, Box<dyn Error>> {
        Ok(self
            .core
            .read_view(|snapshot| Ok::<_, EventError>(self.records.read(snapshot, read)))?)
    }

    fn read_at(
        &self,
        id: u64,
        cursor: Option<ConversationCursor>,
    ) -> Result<Result<AgentConversationResult, RecordsError>, Box<dyn Error>> {
        let mut read = Read::new(terminal(id)?);
        read.cursor = cursor;
        self.read(&read)
    }

    /// The refusal a read of terminal `id` returns.
    fn read_refusal(&self, id: u64) -> Result<ErrorData, Box<dyn Error>> {
        match self.read_at(id, None)? {
            Err(error) => Ok(error.error_data()),
            Ok(page) => Err(format!("expected a refusal, got {page:?}").into()),
        }
    }

    fn page(&self) -> Result<AgentConversationResult, Box<dyn Error>> {
        Ok(self.read_at(AGENT, None)??)
    }
}

/// Record ids of a page, in order.
fn ids(page: &AgentConversationResult) -> Vec<&str> {
    page.records
        .iter()
        .map(|record| record.record_id.as_str())
        .collect()
}

/// The digest an append reports for `records`, recomputed from what a read returns.
fn digest_of(
    records: &[ovis_terminal_protocol::schema::methods::agent::ConversationRecord],
) -> Result<String, Box<dyn Error>> {
    use sha2::{Digest as _, Sha256};
    let mut hasher = Sha256::new();
    for record in records {
        hasher.update(serde_json::to_vec(record)?);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

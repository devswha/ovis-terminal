//! Behavior of agent status over a real store, coordinator and operation ledger.

mod incarnation;
mod machine;
mod osc;
mod rpc;

use std::error::Error;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_core::events::{Coordinator, Event, Subscription};
use ovis_terminal_core::ops::{
    Completion, Context, OperationError, OperationProducer, OperationSnapshot, Operations, Request,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId, TerminalId,
};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::ProcessToken;
use ovis_terminal_protocol::schema::methods::agent::AgentResult;
use ovis_terminal_protocol::schema::methods::server::OpRecord;
use ovis_terminal_protocol::schema::methods::status::StatusResult;
use ovis_terminal_protocol::schema::records::{AgentState, StatusReport};
use ovis_terminal_protocol::schema::scalar::{Incarnation, ReportSeq, TimestampMs};
use ovis_terminal_store::{Snapshot, Store, StoreOptions};
use serde::de::DeserializeOwned;

use super::{AgentStatusEvent, ProcessOwner, Report, StatusService};

type TestResult = Result<(), Box<dyn Error>>;
type Core = Coordinator<AgentStatusEvent, OperationProducer>;

const AGENT: u64 = 4;
const SHELL: u64 = 5;
const AGENT_TOKEN: &str = "token-agent-4";
const SHELL_TOKEN: &str = "token-shell-5";

fn terminal(id: u64) -> Result<TerminalId, Box<dyn Error>> {
    Ok(TerminalId::try_from(id)?)
}

fn token(text: &str) -> ProcessToken {
    ProcessToken::new(text.to_owned())
}

fn now() -> Result<TimestampMs, Box<dyn Error>> {
    Ok(TimestampMs::new(u64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?))
}

struct Harness {
    root: PathBuf,
    core: Core,
    service: StatusService,
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
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000033".parse()?;
    let identity = RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(33)?,
        ServerBootId::try_from(1)?,
    );
    let root = std::env::temp_dir().join(format!("ovis-t33-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&root)?;
    let store = Store::open(&root.join("state"), StoreOptions::default())?;
    let core = Coordinator::new(
        store,
        identity,
        OperationProducer::new(identity.runtime_id()),
    );
    let service = StatusService::new(identity.runtime_id());
    let one = Incarnation::new(1);
    service.tokens().bind(
        &token(AGENT_TOKEN),
        ProcessOwner::new(terminal(AGENT)?, one),
    )?;
    service.tokens().bind(
        &token(SHELL_TOKEN),
        ProcessOwner::new(terminal(SHELL)?, one),
    )?;
    Ok(Harness {
        root,
        core,
        service,
        identity,
    })
}

fn result<T: DeserializeOwned>(record: &OpRecord) -> Result<T, Box<dyn Error>> {
    Ok(serde_json::from_value(
        record.result.clone().ok_or("no result")?,
    )?)
}

fn kind(record: &OpRecord) -> Option<&str> {
    record.error.as_ref().map(|error| error.kind.token())
}

impl Harness {
    /// Runs one operation through accept, start and an atomic commit unit.
    fn run(
        &self,
        build: impl FnOnce(&Snapshot) -> Result<Completion<AgentStatusEvent>, OperationError>,
    ) -> Result<OpRecord, Box<dyn Error>> {
        let id = OpId::new(uuid::Uuid::now_v7().to_string());
        let request = Request::new(id, "status.test".to_owned(), serde_json::json!({}))?;
        let operations = Operations::new(
            &self.core,
            Context::new(PrincipalId::parse("owner")?, self.identity, now()?),
        );
        operations.accept(&request)?;
        let permit = operations.start(request.op_id())?;
        Ok(operations.commit_unit(permit, build)?)
    }

    fn status(&self, id: u64) -> Result<StatusResult, Box<dyn Error>> {
        let terminal = terminal(id)?;
        result(&self.run(|snapshot| self.service.status(snapshot, terminal))?)
    }

    fn agent(&self, id: u64) -> Result<AgentResult, Box<dyn Error>> {
        let terminal = terminal(id)?;
        result(&self.run(|snapshot| self.service.agent(snapshot, terminal))?)
    }

    /// Reports `record` at sequence `seq` with the token `by`.
    fn report(
        &self,
        by: &str,
        (record, seq): (StatusReport, u64),
    ) -> Result<OpRecord, Box<dyn Error>> {
        let mut report = Report::new(terminal(AGENT)?, token(by), record);
        report.seq = ReportSeq::new(seq);
        report.at = now()?;
        self.run(|snapshot| self.service.report(snapshot, report))
    }

    fn state(&self, state: AgentState, seq: u64) -> Result<OpRecord, Box<dyn Error>> {
        self.report(AGENT_TOKEN, (StatusReport::new(state), seq))
    }
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

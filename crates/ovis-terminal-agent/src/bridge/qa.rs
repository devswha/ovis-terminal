//! Behavior of the prompt bridge over a real store, coordinator, operation ledger and
//! prompt service.

mod delivery;
mod sessions;

use std::collections::BTreeMap;
use std::error::Error;
use std::num::NonZeroU8;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_core::events::{Coordinator, EventError};
use ovis_terminal_core::ops::{
    Completion, Context, OperationError, OperationProducer, Operations, Request,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId, TerminalId,
};
use ovis_terminal_protocol::domain::prompt_bridge::ExtensionEvent;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::{ProcessToken, PromptId};
use ovis_terminal_protocol::schema::methods::server::OpRecord;
use ovis_terminal_protocol::schema::methods::status::{
    PromptAnswer, PromptDefinition, PromptField, PromptKind, PromptMechanism,
};
use ovis_terminal_protocol::schema::scalar::{Incarnation, RecordVersion, TimestampMs};
use ovis_terminal_store::{Snapshot, Store, StoreOptions};

use super::{BridgeError, BridgeToken, PromptBridge};
use crate::prompts::{Answer, Open, PromptService};
use crate::status::{AgentStatusEvent, ProcessOwner, StatusService};

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

fn prompt_id(text: &str) -> PromptId {
    PromptId::new(text.to_owned())
}

/// A form with a required `branch` and an optional `note`, answered through `mechanism`.
fn form(mechanism: PromptMechanism) -> PromptDefinition {
    let mut definition =
        PromptDefinition::new(PromptKind::Form, "Release details".to_owned(), mechanism);
    definition.fields = vec![
        PromptField::new("branch".to_owned(), true),
        PromptField::new("note".to_owned(), false),
    ];
    definition
}

fn fields(pairs: &[(&str, &str)]) -> PromptAnswer {
    let values = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()));
    PromptAnswer::Fields(values.collect::<BTreeMap<_, _>>())
}

struct Harness {
    root: PathBuf,
    core: Core,
    status: Arc<StatusService>,
    prompts: Arc<PromptService>,
    bridge: PromptBridge,
    identity: RuntimeIdentity,
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.root) {
            eprintln!("cannot remove {}: {error}", self.root.display());
        }
    }
}

/// A runtime whose agent and shell programs hold their tokens, bridged with `limit`.
fn harness(limit: u8) -> Result<Harness, Box<dyn Error>> {
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000040".parse()?;
    let identity = RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(40)?,
        ServerBootId::try_from(1)?,
    );
    let root = std::env::temp_dir().join(format!("ovis-t40-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&root)?;
    let store = Store::open(&root.join("state"), StoreOptions::default())?;
    let producer = OperationProducer::new(identity.runtime_id());
    let core = Coordinator::new(store, identity, producer);
    let status = Arc::new(StatusService::new(identity.runtime_id()));
    for (id, text) in [(AGENT, AGENT_TOKEN), (SHELL, SHELL_TOKEN)] {
        let owner = ProcessOwner::new(terminal(id)?, Incarnation::new(1));
        status.tokens().bind(&token(text), owner)?;
    }
    let prompts = Arc::new(PromptService::new(
        identity.runtime_id(),
        Arc::clone(&status),
    ));
    let limit = NonZeroU8::new(limit).ok_or("a zero queue limit")?;
    let bridge = PromptBridge::new(Arc::clone(&status), Arc::clone(&prompts), limit);
    Ok(Harness {
        root,
        core,
        status,
        prompts,
        bridge,
        identity,
    })
}

impl Harness {
    /// Runs one operation under `op` through accept, start and an atomic commit unit.
    fn run(
        &self,
        op: &OpId,
        build: impl FnOnce(&Snapshot) -> Result<Completion<AgentStatusEvent>, OperationError>,
    ) -> Result<OpRecord, Box<dyn Error>> {
        let request = Request::new(op.clone(), "prompt.test".to_owned(), serde_json::json!({}))?;
        let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let now = TimestampMs::new(u64::try_from(millis)?);
        let context = Context::new(PrincipalId::parse("owner")?, self.identity, now);
        let operations = Operations::new(&self.core, context);
        operations.accept(&request)?;
        let permit = operations.start(request.op_id())?;
        Ok(operations.commit_unit(permit, build)?)
    }

    /// Opens prompt `id` on terminal `on` as the program holding `by`.
    fn open(
        &self,
        (on, by): (u64, &str),
        (id, definition): (&str, PromptDefinition),
    ) -> Result<OpRecord, Box<dyn Error>> {
        let mut open = Open::new(terminal(on)?, token(by), definition);
        open.prompt_id = Some(prompt_id(id));
        let op = OpId::new(uuid::Uuid::now_v7().to_string());
        self.run(&op, |snapshot| self.prompts.open(snapshot, open))
    }

    /// Answers version `version` of `id` under a fresh operation, returning its id too.
    fn answer(
        &self,
        id: &str,
        (version, answer): (u64, PromptAnswer),
    ) -> Result<(OpId, OpRecord), Box<dyn Error>> {
        let op = OpId::new(uuid::Uuid::now_v7().to_string());
        let place = (prompt_id(id), RecordVersion::new(version));
        let routed = (self.prompts.route(&prompt_id(id))?, op.clone());
        let answer = Answer::new(routed, place, answer);
        let record = self.run(&op, |snapshot| self.prompts.answer(snapshot, answer))?;
        Ok((op, record))
    }

    fn read<T>(&self, read: impl FnOnce(&Snapshot) -> T) -> Result<T, Box<dyn Error>> {
        Ok(self
            .core
            .read_view(|snapshot| Ok::<_, EventError>(read(snapshot)))?)
    }

    fn next(&self, session: &BridgeToken) -> Result<Vec<ExtensionEvent>, Box<dyn Error>> {
        Ok(self.read(|snapshot| self.bridge.next(snapshot, session))??)
    }

    fn try_next(
        &self,
        session: &BridgeToken,
    ) -> Result<Result<Vec<ExtensionEvent>, BridgeError>, Box<dyn Error>> {
        self.read(|snapshot| self.bridge.next(snapshot, session))
    }

    fn ack(
        &self,
        session: &BridgeToken,
        (id, version): (&str, u64),
    ) -> Result<
        Result<ovis_terminal_protocol::domain::prompt_bridge::DeliveryRecord, BridgeError>,
        Box<dyn Error>,
    > {
        let place = (session, &prompt_id(id), RecordVersion::new(version));
        self.read(|snapshot| self.bridge.ack(snapshot, place))
    }
}

fn kind(record: &OpRecord) -> Option<&str> {
    record.error.as_ref().map(|error| error.kind.token())
}

fn ids(events: &[ExtensionEvent]) -> Vec<&str> {
    events
        .iter()
        .map(|event| event.prompt_id.as_str())
        .collect()
}

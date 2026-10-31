//! Behavior of versioned prompts over a real store, coordinator and operation ledger.

mod answers;
mod owners;
mod retention;
mod typed;

use std::error::Error;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_core::events::{Coordinator, Event, EventError, Subscription};
use ovis_terminal_core::ops::{
    Completion, Context, OperationError, OperationProducer, OperationSnapshot, Operations, Request,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId, TerminalId,
};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::{ProcessToken, PromptId};
use ovis_terminal_protocol::schema::methods::server::OpRecord;
use ovis_terminal_protocol::schema::methods::status::{
    PromptAnswer, PromptDefinition, PromptKind, PromptMechanism, PromptRecord,
};
use ovis_terminal_protocol::schema::scalar::{Incarnation, RecordVersion};
use ovis_terminal_store::{Snapshot, Store, StoreOptions};

use super::{Answer, AnswerKeys, KeyInput, KeyInputError, Open, PromptError, PromptService};
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

fn question(mechanism: PromptMechanism) -> PromptDefinition {
    PromptDefinition::new(PromptKind::Question, "Proceed?".to_owned(), mechanism)
}

/// The input path a typed answer reaches, recording every admitted text.
#[derive(Debug, Default)]
struct Keys {
    admitted: Mutex<Vec<AnswerKeys>>,
    refuse: Mutex<Option<KeyInputError>>,
}

impl KeyInput for Keys {
    fn admit(&self, keys: &AnswerKeys) -> Result<(), KeyInputError> {
        let refusal = self
            .refuse
            .lock()
            .map_err(|_| KeyInputError::InputOverflow)?
            .take();
        if let Some(refusal) = refusal {
            return Err(refusal);
        }
        self.admitted
            .lock()
            .map_err(|_| KeyInputError::InputOverflow)?
            .push(keys.clone());
        Ok(())
    }
}

struct Harness {
    root: PathBuf,
    core: Core,
    status: Arc<StatusService>,
    prompts: PromptService,
    keys: Arc<Keys>,
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
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000034".parse()?;
    let identity = RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(34)?,
        ServerBootId::try_from(1)?,
    );
    let root = std::env::temp_dir().join(format!("ovis-t34-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&root)?;
    let store = Store::open(&root.join("state"), StoreOptions::default())?;
    let core = Coordinator::new(
        store,
        identity,
        OperationProducer::new(identity.runtime_id()),
    );
    let status = Arc::new(StatusService::new(identity.runtime_id()));
    for (id, text) in [(AGENT, AGENT_TOKEN), (SHELL, SHELL_TOKEN)] {
        let owner = ProcessOwner::new(terminal(id)?, Incarnation::new(1));
        status.tokens().bind(&token(text), owner)?;
    }
    let keys = Arc::new(Keys::default());
    let prompts = PromptService::new(identity.runtime_id(), Arc::clone(&status))
        .with_keys(Arc::clone(&keys) as Arc<dyn KeyInput>);
    Ok(Harness {
        root,
        core,
        status,
        prompts,
        keys,
        identity,
    })
}

fn result(record: &OpRecord) -> Result<PromptRecord, Box<dyn Error>> {
    Ok(serde_json::from_value(
        record.result.clone().ok_or("no result")?,
    )?)
}

fn kind(record: &OpRecord) -> Option<&str> {
    record.error.as_ref().map(|error| error.kind.token())
}

fn now() -> Result<ovis_terminal_protocol::schema::scalar::TimestampMs, Box<dyn Error>> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    Ok(ovis_terminal_protocol::schema::scalar::TimestampMs::new(
        u64::try_from(millis)?,
    ))
}

impl Harness {
    /// Runs one operation under `op` through accept, start and an atomic commit unit.
    fn run_as(
        &self,
        op: &str,
        build: impl FnOnce(&Snapshot) -> Result<Completion<AgentStatusEvent>, OperationError>,
    ) -> Result<OpRecord, Box<dyn Error>> {
        let request = Request::new(
            OpId::new(op.to_owned()),
            "prompt.test".to_owned(),
            serde_json::json!({}),
        )?;
        let context = Context::new(PrincipalId::parse("owner")?, self.identity, now()?);
        let operations = Operations::new(&self.core, context);
        operations.accept(&request)?;
        let permit = operations.start(request.op_id())?;
        Ok(operations.commit_unit(permit, build)?)
    }

    fn run(
        &self,
        build: impl FnOnce(&Snapshot) -> Result<Completion<AgentStatusEvent>, OperationError>,
    ) -> Result<OpRecord, Box<dyn Error>> {
        self.run_as(&uuid::Uuid::now_v7().to_string(), build)
    }

    /// Opens or republishes `id` on the agent terminal as the program holding `by`.
    fn open(
        &self,
        by: &str,
        (id, definition): (&str, PromptDefinition),
    ) -> Result<OpRecord, Box<dyn Error>> {
        let mut open = Open::new(terminal(AGENT)?, token(by), definition);
        open.prompt_id = Some(prompt_id(id));
        self.run(|snapshot| self.prompts.open(snapshot, open))
    }

    fn answer(
        &self,
        id: &str,
        (version, answer): (u64, PromptAnswer),
    ) -> Result<OpRecord, Box<dyn Error>> {
        let op = uuid::Uuid::now_v7().to_string();
        let place = (prompt_id(id), RecordVersion::new(version));
        let routed = (self.prompts.route(&prompt_id(id))?, OpId::new(op.clone()));
        let answer = Answer::new(routed, place, answer);
        self.run_as(&op, |snapshot| self.prompts.answer(snapshot, answer))
    }

    fn get(&self, id: &str) -> Result<PromptRecord, Box<dyn Error>> {
        Ok(self.read(|snapshot| self.prompts.get(snapshot, &prompt_id(id)))??)
    }

    fn admission(&self, id: u64) -> Result<Result<(), PromptError>, Box<dyn Error>> {
        let terminal = terminal(id)?;
        self.read(|snapshot| self.prompts.admission(snapshot, terminal))
    }

    fn read<T>(&self, read: impl FnOnce(&Snapshot) -> T) -> Result<T, Box<dyn Error>> {
        Ok(self
            .core
            .read_view(|snapshot| Ok::<_, EventError>(read(snapshot)))?)
    }
}

fn text(answer: &str) -> PromptAnswer {
    PromptAnswer::Text(answer.to_owned())
}

/// Domain events already delivered to `subscription`, skipping operation receipts.
fn domain_events(
    subscription: &Subscription<AgentStatusEvent, OperationSnapshot>,
) -> Vec<Event<AgentStatusEvent>> {
    std::iter::from_fn(|| subscription.delivery.try_recv().ok())
        .filter(|event| !matches!(event.data, AgentStatusEvent::Operation(_)))
        .collect()
}

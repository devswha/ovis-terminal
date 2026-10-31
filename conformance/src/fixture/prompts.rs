//! The prompt lane over the fixture's agent terminals, and the byte count of what reached
//! each fixture program's input.
//!
//! `fixture.pty` stands in for reading the PTY: it reports how many bytes `input.send`
//! echoed and typed answers admitted per terminal. Ordinary sends consult the prompt
//! service first, so a blocked program receives no byte.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ovis_terminal_agent::prompts::{AnswerKeys, KeyInput, KeyInputError, PromptService};
use ovis_terminal_agent::status::StatusService;
use ovis_terminal_core::registry::{RuntimeIdentity, TerminalId};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::methods::status::StatusTargetParams;
use ovis_terminal_server::dispatch::lanes::agent_status::TerminalDirectory;
use ovis_terminal_server::dispatch::lanes::prompts::{
    AnswerPrompt, ClosePrompt, GetPrompt, ListPrompts, OpenPrompt, PromptLane,
};
use serde_json::{Value, json};

use super::agents::Directory;
use super::handler::Dispatch;
use super::serve::at;
use crate::error::RunnerError;

/// Bytes that reached each fixture program's input.
#[derive(Debug, Default)]
pub(crate) struct Pty(Mutex<BTreeMap<TerminalId, u64>>);

impl Pty {
    pub(crate) fn wrote(&self, terminal: TerminalId, bytes: usize) -> Result<(), KeyInputError> {
        let bytes = u64::try_from(bytes).map_err(|_| KeyInputError::InputOverflow)?;
        let mut written = self.0.lock().map_err(|_| KeyInputError::InputOverflow)?;
        let total = written.entry(terminal).or_insert(0);
        *total = total.saturating_add(bytes);
        drop(written);
        Ok(())
    }

    fn bytes(&self, terminal: TerminalId) -> Option<u64> {
        self.0
            .lock()
            .ok()
            .map(|written| written.get(&terminal).copied().unwrap_or(0))
    }
}

/// Typed answers reach the fixture program's input like any other admitted text.
#[derive(Debug)]
struct Typed(Arc<Pty>);

impl KeyInput for Typed {
    fn admit(&self, keys: &AnswerKeys) -> Result<(), KeyInputError> {
        self.0.wrote(keys.owner.terminal, keys.text.len())
    }
}

/// The prompt service whose open prompts gate the fixture's `input.send`.
#[derive(Debug)]
pub(crate) struct Gate {
    pub(crate) prompts: Arc<PromptService>,
    pub(crate) pty: Arc<Pty>,
}

impl Gate {
    pub(crate) fn new(runtime: RuntimeIdentity, status: &Arc<StatusService>) -> Arc<Self> {
        let pty = Arc::new(Pty::default());
        let keys: Arc<dyn KeyInput> = Arc::new(Typed(Arc::clone(&pty)));
        let prompts = PromptService::new(runtime.runtime_id(), Arc::clone(status)).with_keys(keys);
        Arc::new(Self {
            prompts: Arc::new(prompts),
            pty,
        })
    }
}

/// Registers every `prompt.*` method over the agent terminals.
pub(crate) fn register(
    dispatch: &mut Dispatch,
    (gate, directory): (&Gate, Arc<Directory>),
    runtime: RuntimeIdentity,
) -> Result<(), RunnerError> {
    let lane = PromptLane::new(directory, Arc::clone(&gate.prompts), runtime.machine_id());
    dispatch
        .register(OpenPrompt::new(&lane))
        .map_err(at("register prompt.open"))?;
    dispatch
        .register(AnswerPrompt::new(&lane))
        .map_err(at("register prompt.answer"))?;
    dispatch
        .register(ClosePrompt::new(&lane))
        .map_err(at("register prompt.close"))?;
    dispatch
        .register_read(GetPrompt::new(&lane))
        .map_err(at("register prompt.get"))?;
    dispatch
        .register_read(ListPrompts::new(&lane))
        .map_err(at("register prompt.list"))
}

/// What reached the PTY input of the named fixture terminal.
pub(crate) fn pty(
    (gate, directory): (&Gate, &Directory),
    request: &RpcRequest,
) -> Result<Value, BoundaryError> {
    let params: StatusTargetParams = decode_params(request).map_err(BoundaryError::from)?;
    let resolved = directory
        .resolve(&params.target)
        .map_err(|_| BoundaryError::invalid_params(Some("params.target")))?;
    let bytes = gate
        .pty
        .bytes(resolved.terminal)
        .ok_or_else(|| BoundaryError::invalid_params(Some("params.target")))?;
    Ok(json!({ "bytes": bytes.to_string() }))
}

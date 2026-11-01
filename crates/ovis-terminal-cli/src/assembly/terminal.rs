//! `terminal.run`, `terminal.list`, `terminal.get`, `op.get` and `server.status` on the
//! actor thread, over the landed lifecycle service.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ovis_terminal_agent::status::ProcessOwner;
use ovis_terminal_authz::Session;
use ovis_terminal_core::registry::{ResourceName, TerminalId};
use ovis_terminal_core::terminals::{Call, Launch, LaunchState, Program, TerminalError};
use ovis_terminal_protocol::domain::boundary_policy::{LaunchArgv, LaunchEnvironment};
use ovis_terminal_protocol::errors::{BoundaryError, OpId};
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::ids::ProcessToken;
use ovis_terminal_protocol::schema::methods::terminal::{LaunchProgram, TerminalRunParams};
use ovis_terminal_protocol::schema::scalar::Incarnation;
use ovis_terminal_server::dispatch::lanes::agent_status::TerminalDirectory as _;
use serde_json::{Value, json};

use super::actor::Runtime;
use super::error::refusal;

/// Bound on waiting for the holder's spawn evidence before answering `pending`.
const SPAWN_BOUND: Duration = Duration::from_secs(10);

/// The typed refusal of a lifecycle failure.
pub(super) fn terminal_refusal(error: &TerminalError) -> BoundaryError {
    refusal(error.kind(), error.hint())
}

/// The environment variable a spawned program finds its process token in.
const PROCESS_TOKEN_VAR: &str = "OVIS_TERMINAL_PROCESS_TOKEN";

fn launch(params: TerminalRunParams) -> Result<(Launch, ProcessToken), TerminalError> {
    let unsupported = params.parent.is_some()
        || params.child_policy.is_some()
        || params.first_message.is_some()
        || params.machine.is_some();
    // `program` is required in this protocol revision and optional beside `template` in
    // the next; its serialized form decodes as `Option<LaunchProgram>` from either.
    let program: Option<LaunchProgram> =
        serde_json::from_value(serde_json::to_value(&params.program)?)?;
    let Some(LaunchProgram::Argv(argv)) = program else {
        return Err(TerminalError::UnsupportedField);
    };
    if unsupported {
        return Err(TerminalError::UnsupportedField);
    }
    let cwd = match params.cwd {
        Some(cwd) => PathBuf::from(cwd),
        None => std::env::current_dir().map_err(TerminalError::Cwd)?,
    };
    let mut env: BTreeMap<String, String> = params.env.unwrap_or_default();
    let token = ProcessToken::new(uuid::Uuid::new_v4().simple().to_string());
    env.insert(PROCESS_TOKEN_VAR.to_owned(), token.as_str().to_owned());
    let mut launch = Launch::new(Program::new(argv, cwd, env)?, params.geometry)?;
    launch.name = params
        .name
        .map(|name| ResourceName::new(name.as_str()))
        .transpose()?;
    launch.labels = params.labels.unwrap_or_default();
    launch.keep = params.keep.unwrap_or(false);
    Ok((launch, token))
}

/// Refuses an oversized or unsafe launch before anything is admitted (plan todo 58): the
/// argv and environment bounds, NUL bytes and runtime-reserved variables.
fn bounded(params: &TerminalRunParams) -> Result<(), BoundaryError> {
    if let Some(LaunchProgram::Argv(argv)) = &params.program {
        LaunchArgv::parse(argv).map_err(|violation| violation.to_boundary())?;
    }
    if let Some(env) = &params.env {
        LaunchEnvironment::parse(env).map_err(|violation| violation.to_boundary())?;
    }
    Ok(())
}

impl Runtime {
    pub(super) fn resolve_id(&self, target: &Target) -> Result<TerminalId, BoundaryError> {
        match target {
            Target::TerminalId(id) => id
                .as_str()
                .parse()
                .map_err(|_| refusal("not_found", "name a terminal_id from terminal.list")),
            _ => self
                .directory
                .resolve(target)
                .map(|resolved| resolved.terminal)
                .map_err(|error| refusal(error.kind(), error.hint())),
        }
    }

    /// Admits the launch, then follows holder messages until spawn evidence or the bound.
    pub(super) fn run_terminal(
        &mut self,
        session: &Session,
        params: TerminalRunParams,
    ) -> Result<Value, BoundaryError> {
        bounded(&params)?;
        let op_id: OpId = params.op_id.clone();
        let grid = params.geometry;
        let (launch, token) = launch(params).map_err(|error| terminal_refusal(&error))?;
        let name = launch.name.as_ref().map(|name| name.as_str().to_owned());
        let record =
            self.lifecycle(|terminals| terminals.run(Call::new(session, op_id), launch))?;
        let id = record.id();
        self.created.entry(id).or_insert(record.revision);
        if !self.live.contains_key(&id) {
            self.launches.insert(id, (grid, name));
        }
        let deadline = Instant::now().checked_add(SPAWN_BOUND);
        let mut record = record;
        while matches!(record.spawn, LaunchState::Admitted | LaunchState::Pending)
            && deadline.is_some_and(|deadline| Instant::now() < deadline)
        {
            if !self.receive(Duration::from_millis(50)) {
                break;
            }
            record = self.lifecycle(|terminals| terminals.get(session, id))?;
        }
        if let Some(process) = record.lifecycle.process() {
            let owner = ProcessOwner::new(id, Incarnation::new(process.generation().get()));
            self.status
                .tokens()
                .bind(&token, owner)
                .map_err(|error| refusal(error.kind(), error.hint()))?;
        }
        let spawn = match record.spawn {
            LaunchState::Spawned => "spawned",
            LaunchState::Failed => "failed",
            _ => "pending",
        };
        let created = self.created.get(&id).copied().unwrap_or(record.revision);
        let terminal = self.record_value(&record)?;
        Ok(json!({"terminal": terminal, "spawn": spawn, "created_seq": created}))
    }

    pub(super) fn list_terminals(&self, session: &Session) -> Result<Value, BoundaryError> {
        let records = self.lifecycle(|terminals| terminals.list(session))?;
        let terminals = records
            .iter()
            .map(|record| self.record_value(record))
            .collect::<Result<Vec<Value>, BoundaryError>>()?;
        let count = terminals.len().to_string();
        Ok(json!({
            "terminals": terminals,
            "page": {"next_cursor": null, "total": count},
            "retention_floor": "0",
        }))
    }

    pub(super) fn get_terminal(
        &self,
        session: &Session,
        target: &Target,
    ) -> Result<Value, BoundaryError> {
        let id = self.resolve_id(target)?;
        let record = self.lifecycle(|terminals| terminals.get(session, id))?;
        self.record_value(&record)
    }

    pub(super) fn operation(
        &self,
        session: &Session,
        op_id: &OpId,
    ) -> Result<Value, BoundaryError> {
        let record = self.lifecycle(|terminals| terminals.operation(session, op_id))?;
        serde_json::to_value(record).map_err(|_| refusal("internal", "report the build"))
    }
}

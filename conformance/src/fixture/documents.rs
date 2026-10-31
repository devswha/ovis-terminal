//! `doc.set` and `doc.get` registered on the real dispatcher, with file effects the runner
//! inspects directly. The document named `held` never commits until it is cancelled.

use std::path::{Path, PathBuf};

use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_core::registry::{MachineId, RuntimeIdentity, TargetResolutionError};
use ovis_terminal_protocol::schema::ids::DocName;
use ovis_terminal_protocol::schema::methods::doc::{DocGetParams, DocSetParams};
use ovis_terminal_server::dispatch::{Access, Completed, DispatchError, Execution, Method};
use serde_json::{Value, json};

/// The document whose write waits for cancellation, to hold an operation open.
pub(crate) const HELD: &str = "held";
/// Sent on the request's connection once the held write runs, i.e. after durable accept;
/// scenarios await it before inspecting or cancelling instead of sleeping.
pub(crate) const HELD_NOTIFICATION: &str = "$/fixture/held";

/// The effect file of one document; only plain names map to a file.
fn effect(effects: &Path, name: &DocName) -> Result<PathBuf, DispatchError> {
    let plain = !name.as_str().is_empty()
        && name
            .as_str()
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if !plain {
        return Err(TargetResolutionError::NotFound.into());
    }
    Ok(effects.join(format!("{}.json", name.as_str())))
}

fn access(machine: MachineId, action: Action, name: &DocName) -> Access {
    Access::new(
        action,
        ResolvedTarget::new(machine, TargetResource::Document { name: name.clone() }),
    )
}

/// Writes `<effects>/<name>.json` inside the authorized commit boundary.
#[derive(Debug)]
pub(crate) struct DocSet {
    effects: PathBuf,
    machine: MachineId,
}

impl DocSet {
    pub(crate) fn new(effects: &Path, runtime: RuntimeIdentity) -> Self {
        Self {
            effects: effects.to_path_buf(),
            machine: runtime.machine_id(),
        }
    }
}

impl Method<OperationEvent, OperationProducer> for DocSet {
    const NAME: &'static str = "doc.set";
    type Params = DocSetParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        effect(&self.effects, &params.name)?;
        Ok(access(self.machine, Action::DocumentWrite, &params.name))
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<OperationEvent, OperationProducer>,
    ) -> Result<Completed, DispatchError> {
        if params.name.as_str() == HELD {
            let op_id = execution.op_id().clone();
            execution
                .notifier()
                .notify(HELD_NOTIFICATION, json!({ "op_id": op_id }))
                .await
                .map_err(std::io::Error::other)?;
            execution.cancelled().await;
        }
        let path = effect(&self.effects, &params.name)?;
        let name = params.name;
        let value = params.value;
        execution
            .commit(move || {
                std::fs::write(path, serde_json::to_vec(&value)?)?;
                Ok(json!({ "name": name, "value": value }))
            })
            .await
    }
}

/// Reads `<effects>/<name>.json`; an absent file is a null value.
#[derive(Debug)]
pub(crate) struct DocGet {
    effects: PathBuf,
    machine: MachineId,
}

impl DocGet {
    pub(crate) fn new(effects: &Path, runtime: RuntimeIdentity) -> Self {
        Self {
            effects: effects.to_path_buf(),
            machine: runtime.machine_id(),
        }
    }
}

impl Method<OperationEvent, OperationProducer> for DocGet {
    const NAME: &'static str = "doc.get";
    type Params = DocGetParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        effect(&self.effects, &params.name)?;
        Ok(access(self.machine, Action::Read, &params.name))
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<OperationEvent, OperationProducer>,
    ) -> Result<Completed, DispatchError> {
        let path = effect(&self.effects, &params.name)?;
        let name = params.name;
        execution
            .commit(move || {
                let value: Value = match std::fs::read(&path) {
                    Ok(bytes) => serde_json::from_slice(&bytes)?,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Value::Null,
                    Err(error) => return Err(error.into()),
                };
                Ok(json!({ "name": name, "value": value }))
            })
            .await
    }
}

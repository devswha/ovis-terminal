//! `doc.get` and `doc.set` over the core document service (plan todo 50).
//!
//! `doc.set` is one coordinator unit: the compare-and-swap, its `doc.changed` event and the
//! operation receipt commit together, and a rejected write is retained as the operation's
//! typed failure. `doc.get` is an unrecorded read of one committed snapshot: no receipt, no
//! events, so a document of any accepted size reads back.

use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::docs::{self, DocChanged, set_unit};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_core::registry::{MachineId, RuntimeIdentity, TargetResolutionError};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::common::MachineRef;
use ovis_terminal_protocol::schema::ids::DocName;
use ovis_terminal_protocol::schema::methods::doc::{DocGetParams, DocSetParams};
use ovis_terminal_store::Snapshot;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::dispatch::{Access, Completed, DispatchError, Dispatcher, Execution, Method, Read};

/// `doc.set`: compare-and-swap of one document on this machine.
#[derive(Debug)]
pub struct DocSet {
    machine: MachineId,
}

/// `doc.get`: one document read at one snapshot on this machine.
#[derive(Debug)]
pub struct DocGet {
    machine: MachineId,
}

/// Resolves the document on this machine; another machine is never silently substituted.
fn access(
    machine: MachineId,
    requested: Option<&MachineRef>,
    (action, name): (Action, &DocName),
) -> Result<Access, DispatchError> {
    match requested {
        None => {}
        Some(MachineRef::MachineId(id)) if id.as_str() == machine.to_string() => {}
        Some(_) => return Err(TargetResolutionError::NotFound.into()),
    }
    let target = TargetResource::Document { name: name.clone() };
    Ok(Access::new(action, ResolvedTarget::new(machine, target)))
}

impl<D, P> Method<D, P> for DocSet
where
    D: EventData + From<OperationEvent> + From<DocChanged> + Clone + Serialize + DeserializeOwned,
    D: Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "doc.set";
    type Params = DocSetParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        let document = (Action::DocumentWrite, &params.name);
        access(self.machine, params.machine.as_ref(), document)
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        execution
            .commit_unit(move |snapshot| Ok(set_unit(params, snapshot)))
            .await
    }
}

impl Read for DocGet {
    const NAME: &'static str = "doc.get";
    type Params = DocGetParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        access(
            self.machine,
            params.machine.as_ref(),
            (Action::Read, &params.name),
        )
    }

    fn read(&self, params: Self::Params, snapshot: &Snapshot) -> Result<Value, BoundaryError> {
        docs::read(snapshot, &params.name)
            .and_then(|record| Ok(serde_json::to_value(record)?))
            .map_err(|error| BoundaryError::Domain {
                data: Box::new(error.error_data()),
            })
    }
}

/// Registers `doc.get` and `doc.set` for documents stored by `runtime`'s machine.
///
/// # Errors
///
/// Returns `method_not_found` when either method is already registered.
pub fn register<D, P>(
    dispatch: &mut Dispatcher<D, P>,
    runtime: RuntimeIdentity,
) -> Result<(), DispatchError>
where
    D: EventData + From<OperationEvent> + From<DocChanged> + Clone + Serialize + DeserializeOwned,
    D: Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    let machine = runtime.machine_id();
    dispatch.register_read(DocGet { machine })?;
    dispatch.register(DocSet { machine })
}

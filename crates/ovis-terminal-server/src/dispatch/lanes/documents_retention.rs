//! `doc.get` and `doc.set` over the core document service (plan todo 50).
//!
//! Each call is one coordinator unit: the read or compare-and-swap, its `doc.changed` event
//! and the operation receipt commit together, and a rejected write is retained as the
//! operation's typed failure.

use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::docs::{DocChanged, get_unit, set_unit};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_core::registry::{MachineId, RuntimeIdentity, TargetResolutionError};
use ovis_terminal_protocol::schema::common::MachineRef;
use ovis_terminal_protocol::schema::ids::DocName;
use ovis_terminal_protocol::schema::methods::doc::{DocGetParams, DocSetParams};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::dispatch::{Access, Completed, DispatchError, Dispatcher, Execution, Method};

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

impl<D, P> Method<D, P> for DocGet
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "doc.get";
    type Params = DocGetParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        access(
            self.machine,
            params.machine.as_ref(),
            (Action::Read, &params.name),
        )
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        execution
            .commit_unit(move |snapshot| Ok(get_unit(&params.name, snapshot)))
            .await
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
    dispatch.register(DocGet { machine })?;
    dispatch.register(DocSet { machine })
}

//! `plugin.list`, `plugin.invoke` and `plugin.run_get` over the plugin service (todo 46).
//!
//! Invocation is authorized as `plugin_invoke` on the named plugin before the catalog is
//! read, so an unauthorized caller learns nothing and no program starts. The program runs
//! inside the operation's commit boundary: the ledger stays `running` while it runs and
//! commits the settled run record, including a failing exit, as the operation's receipt.

use std::sync::Arc;

use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationError, OperationEvent, OperationSnapshot, Operations};
use ovis_terminal_core::registry::{MachineId, TargetResolutionError};
use ovis_terminal_protocol::schema::common::NoParams;
use ovis_terminal_protocol::schema::ids::PluginRunId;
use ovis_terminal_protocol::schema::methods::plugin::{
    PluginInvokeParams, PluginRunGetParams, PluginRunRecord,
};
use ovis_terminal_protocol::schema::methods::server::OpState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use ovis_terminal_services::plugins::{PluginError, Plugins};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::dispatch::{Access, Completed, DispatchError, Execution, Method};

#[cfg(all(test, unix))]
mod tests;

fn refused(error: PluginError) -> DispatchError {
    DispatchError::Domain {
        kind: error.kind(),
        hint: error.hint(),
        source: Box::new(error),
    }
}

/// `plugin.list`: the catalog with every action's unavailability reason.
#[derive(Debug)]
pub struct PluginList {
    plugins: Arc<Plugins>,
    machine: MachineId,
}

/// `plugin.invoke`: one authorized, durable, bounded run of a declared program.
#[derive(Debug)]
pub struct PluginInvoke {
    plugins: Arc<Plugins>,
    machine: MachineId,
}

/// `plugin.run_get`: a run of the caller's own invocations, live or committed.
#[derive(Debug)]
pub struct PluginRunGet {
    plugins: Arc<Plugins>,
    machine: MachineId,
}

/// The three plugin routes over one shared service, for registration.
pub fn routes(
    plugins: &Arc<Plugins>,
    machine: MachineId,
) -> (PluginList, PluginInvoke, PluginRunGet) {
    (
        PluginList {
            plugins: Arc::clone(plugins),
            machine,
        },
        PluginInvoke {
            plugins: Arc::clone(plugins),
            machine,
        },
        PluginRunGet {
            plugins: Arc::clone(plugins),
            machine,
        },
    )
}

impl<D, P> Method<D, P> for PluginList
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "plugin.list";
    type Params = NoParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(Access::new(
            Action::Read,
            ResolvedTarget::new(self.machine, TargetResource::Machine),
        ))
    }

    async fn run(
        &self,
        _: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        let plugins = Arc::clone(&self.plugins);
        let listing =
            tokio::task::spawn_blocking(move || plugins.catalog().map(|found| found.listing()))
                .await?
                .map_err(refused)?;
        execution.commit(move || Ok(listing)).await
    }
}

impl<D, P> Method<D, P> for PluginInvoke
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "plugin.invoke";
    type Params = PluginInvokeParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(Access::new(
            Action::PluginInvoke,
            ResolvedTarget::new(
                self.machine,
                TargetResource::Plugin {
                    plugin: params.plugin_id.clone(),
                },
            ),
        ))
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        let run_id = PluginRunId::of_operation(execution.op_id());
        let plugins = Arc::clone(&self.plugins);
        let prepared = tokio::task::spawn_blocking(move || plugins.prepare(run_id, &params))
            .await?
            .map_err(refused)?;
        let plugins = Arc::clone(&self.plugins);
        let cancel = execution.cancellation.clone();
        let runtime = tokio::runtime::Handle::current();
        execution
            .commit(move || {
                runtime
                    .block_on(plugins.run(prepared, cancel))
                    .map_err(refused)
            })
            .await
    }
}

impl<D, P> Method<D, P> for PluginRunGet
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "plugin.run_get";
    type Params = PluginRunGetParams;

    fn resolve(&self, params: &Self::Params, session: &Session) -> Result<Access, DispatchError> {
        let op_id = params
            .run_id
            .operation()
            .ok_or(TargetResolutionError::NotFound)?;
        Ok(Access::new(
            Action::InspectOperation,
            ResolvedTarget::new(
                self.machine,
                TargetResource::Operation {
                    principal: session.principal().clone(),
                    op_id,
                },
            ),
        ))
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        let op_id = params
            .run_id
            .operation()
            .ok_or(TargetResolutionError::NotFound)?;
        let core = Arc::clone(&execution.state.core);
        let context = execution.context.clone();
        let plugins = Arc::clone(&self.plugins);
        execution
            .commit(move || {
                // The ledger is principal-scoped: another client's run is simply not found.
                let record = Operations::new(&core, context).get(&op_id)?;
                if record.method.as_str() != "plugin.invoke" {
                    return Err(OperationError::NotFound.into());
                }
                match (&record.state, &record.result) {
                    (OpenEnum::Known(OpState::Committed), Some(result)) => {
                        Ok(serde_json::from_value::<PluginRunRecord>(result.clone())?)
                    }
                    (OpenEnum::Known(OpState::Running), _) => plugins
                        .live_record(&params.run_id)
                        .map_err(refused)?
                        .ok_or_else(|| OperationError::Settled(Box::new(record)).into()),
                    _ => Err(OperationError::Settled(Box::new(record)).into()),
                }
            })
            .await
    }
}

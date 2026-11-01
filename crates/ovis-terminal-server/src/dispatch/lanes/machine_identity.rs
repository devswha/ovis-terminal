//! `machine.add`, `machine.get` and `machine.rename` over the remote identity registry.
//!
//! Every method acts on this runtime's machine administration: adding and renaming need the
//! administer grant, reading needs observe. A reference is resolved to a stable machine
//! identity once, before the commit; the commit uses only that identity.

mod add;

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

pub use add::MachineAdd;
use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_core::registry::MachineId;
use ovis_terminal_protocol::schema::common::MachineRef;
use ovis_terminal_protocol::schema::methods::machine::{
    MachineRecord, MachineRefParams, MachineRenameParams,
};
use ovis_terminal_remote::identity::{IdentityError, Machines, Name};
use ovis_terminal_remote::transport::{Link, TransportError};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::dispatch::{Access, Completed, DispatchError, Execution, Method};

impl From<IdentityError> for DispatchError {
    fn from(error: IdentityError) -> Self {
        Self::Domain {
            kind: error.kind(),
            hint: error.hint(),
            source: Box::new(error),
        }
    }
}

impl From<TransportError> for DispatchError {
    fn from(error: TransportError) -> Self {
        Self::Domain {
            kind: error.kind(),
            hint: error.hint(),
            source: Box::new(error),
        }
    }
}

/// The machines this runtime administers and the bound on each handshake.
#[derive(Debug)]
pub struct MachineLane {
    local: MachineId,
    machines: Mutex<Machines<Option<Link>>>,
    handshake_bound: Duration,
}

impl MachineLane {
    /// Machines administered by the runtime of `local`, each handshake bounded by `bound`.
    #[must_use]
    pub fn new(local: MachineId, bound: Duration) -> Arc<Self> {
        Arc::new(Self {
            local,
            machines: Mutex::new(Machines::default()),
            handshake_bound: bound,
        })
    }

    const fn access(&self, action: Action) -> Access {
        Access::new(
            action,
            ResolvedTarget::new(self.local, TargetResource::Machine),
        )
    }

    fn machines(&self) -> Result<MutexGuard<'_, Machines<Option<Link>>>, DispatchError> {
        self.machines
            .lock()
            .map_err(|_| std::io::Error::other("the machine registry is poisoned").into())
    }

    /// The record of an added machine and its link, which leaves the registry so that the
    /// machine's actor can own it; `None` once it was taken.
    ///
    /// # Errors
    ///
    /// The registry failure, such as `not_found` for a machine never added.
    pub fn take_link(
        &self,
        machine: &MachineRef,
    ) -> Result<(MachineRecord, Option<Link>), DispatchError> {
        let mut machines = self.machines()?;
        let binding = machines.bind(machine)?;
        let record = machines.get(&binding)?;
        Ok((record, machines.link_mut(&binding)?.take()))
    }
}

/// `machine.get`: one machine by identity, name or alias.
#[derive(Debug)]
pub struct MachineGet(Arc<MachineLane>);

impl MachineGet {
    /// Reads machines of `lane`.
    #[must_use]
    pub fn new(lane: &Arc<MachineLane>) -> Self {
        Self(Arc::clone(lane))
    }
}

impl<D, P> Method<D, P> for MachineGet
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "machine.get";
    type Params = MachineRefParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(self.0.access(Action::Read))
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        let binding = self.0.machines()?.bind(&params.machine)?;
        let lane = Arc::clone(&self.0);
        execution
            .commit(move || Ok(lane.machines()?.get(&binding)?))
            .await
    }
}

/// `machine.rename`: a new name; the old one stays an alias of the same machine.
#[derive(Debug)]
pub struct MachineRename(Arc<MachineLane>);

impl MachineRename {
    /// Renames machines of `lane`.
    #[must_use]
    pub fn new(lane: &Arc<MachineLane>) -> Self {
        Self(Arc::clone(lane))
    }
}

impl<D, P> Method<D, P> for MachineRename
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "machine.rename";
    type Params = MachineRenameParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(self.0.access(Action::Administer))
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        let name = Name::try_from(&params.name)?;
        let binding = {
            let machines = self.0.machines()?;
            let binding = machines.bind(&params.machine)?;
            machines.ensure_renamable(&binding, &name)?;
            binding
        };
        let lane = Arc::clone(&self.0);
        execution
            .commit(move || Ok(lane.machines()?.rename(&binding, name)?))
            .await
    }
}

#[cfg(all(test, unix))]
mod qa;

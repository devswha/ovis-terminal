//! Service lanes composed onto one dispatcher (plan todo 52).
//!
//! A served runtime registers its services through [`ServiceLanes`] so that every service
//! method is routed by the same dispatcher, authorized against the same machine scope and
//! recorded in the same operation ledger as terminal work. Lanes whose events need their
//! own event domain (worktrees, documents) register through their own lane modules.

use std::sync::Arc;

use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_core::registry::MachineId;
use ovis_terminal_services::plugins::Plugins;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::lanes::plugins::routes;
use super::lanes::update::UpdateLane;
use super::{DispatchError, Dispatcher};

/// The service lanes one runtime serves on `machine`.
#[derive(Debug)]
pub struct ServiceLanes {
    machine: MachineId,
    plugins: Option<Arc<Plugins>>,
    update: Option<UpdateLane>,
}

impl ServiceLanes {
    /// No services yet, for the runtime on `machine`.
    #[must_use]
    pub const fn new(machine: MachineId) -> Self {
        Self {
            machine,
            plugins: None,
            update: None,
        }
    }

    /// Serves `plugin.list`, `plugin.invoke` and `plugin.run_get` over `plugins`.
    #[must_use]
    pub fn with_plugins(mut self, plugins: Arc<Plugins>) -> Self {
        self.plugins = Some(plugins);
        self
    }

    /// Serves the `update.*` methods of `lane`.
    #[must_use]
    pub fn with_update(mut self, lane: UpdateLane) -> Self {
        self.update = Some(lane);
        self
    }

    /// Registers every configured service.
    ///
    /// # Errors
    ///
    /// A registration failure when a method is already registered; nothing after it is
    /// registered.
    pub fn register<D, P>(self, dispatcher: &mut Dispatcher<D, P>) -> Result<(), DispatchError>
    where
        D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
        P: SnapshotProducer + Send + Sync + 'static,
        P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
    {
        if let Some(plugins) = &self.plugins {
            let (list, invoke, run_get) = routes(plugins, self.machine);
            dispatcher.register(list)?;
            dispatcher.register(invoke)?;
            dispatcher.register(run_get)?;
        }
        self.update
            .map_or_else(|| Ok(()), |update| update.register(dispatcher))
    }
}

#[cfg(test)]
mod tests;
